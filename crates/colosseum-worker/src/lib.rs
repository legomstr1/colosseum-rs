//! The Cloudflare Worker for online play (see `docs/ONLINE.md`).
//!
//! Static files (the page and the WebAssembly engine, built into `dist/`)
//! are served by Workers Static Assets and never reach this code. The
//! Worker handles:
//! - `POST /api/online/games {seats}`: makes a game code and sets up its
//!   Durable Object.
//! - `GET /ws/{code}`: a WebSocket to that game's Durable Object.
//!
//! Each game is one [`GameRoom`]. It keeps the seed, the seats, who holds
//! which seat, and the ordered moves. It checks every move with the engine
//! and relays accepted ones to every browser. Browsers run the engine
//! themselves, and the one on bot duty sends the bots' moves.

use colosseum_engine::{Action, Game, GameConfig};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::Arc;
use worker::wasm_bindgen::prelude::*;
use worker::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = crypto, js_name = randomUUID)]
    fn random_uuid() -> String;
    #[wasm_bindgen(js_namespace = Date)]
    fn now() -> f64;
}

/// 122 random bits as 32 hex digits.
fn random_hex() -> String {
    random_uuid().replace('-', "")
}

/// A short game code in Crockford base32 (no I, L, O, U).
fn game_code() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let bits = u128::from_str_radix(&random_hex(), 16).unwrap_or(0);
    (0..6)
        .map(|i| ALPHABET[((bits >> (5 * i)) & 31) as usize] as char)
        .collect()
}

/// Seat kinds online play offers. Bots run in a player's browser, which has
/// only live-input built in.
const HUMAN: &str = "human";
const BOT_KINDS: &[&str] = &["live-input"];

#[derive(Clone, Serialize, Deserialize)]
struct Meta {
    seats: Vec<String>,
    seed: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Claim {
    name: String,
    token: String,
}

/// What a WebSocket is, kept on the socket so it survives hibernation.
#[derive(Clone, Serialize, Deserialize)]
struct Attachment {
    seat: Option<usize>,
    /// When it joined (ms), to pick the bot runner.
    joined: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ClientMsg {
    Join {
        #[serde(default)]
        name: String,
        seat: Option<usize>,
        token: Option<String>,
    },
    Action {
        index: usize,
        action: Action,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum ServerMsg<'a> {
    Hello {
        seats: &'a [String],
        seed: u64,
        history: &'a [Action],
        names: Vec<String>,
        you: You,
    },
    Seats {
        names: Vec<String>,
        connected: Vec<bool>,
        bot_runner: Option<usize>,
    },
    Applied {
        index: usize,
        action: &'a Action,
    },
    Error {
        message: String,
    },
}

#[derive(Serialize)]
struct You {
    seat: Option<usize>,
    token: Option<String>,
}

#[derive(Deserialize)]
struct NewGame {
    seats: Vec<String>,
}

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let path = req.path();
    if req.method() == Method::Post && path == "/api/online/games" {
        let body: NewGame = match req.json().await {
            Ok(b) => b,
            Err(_) => return json_error(400, "bad request"),
        };
        if let Err(e) = check_seats(&body.seats) {
            return json_error(400, &e);
        }
        // 52 bits: seeds pass through JavaScript numbers (JSON, storage).
        let seed = u64::from_str_radix(&random_hex()[..13], 16).unwrap_or(1);
        let code = game_code();
        let meta = Meta {
            seats: body.seats,
            seed,
        };
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_body(Some(serde_json::to_string(&meta)?.into()));
        let inner = Request::new_with_init("https://room/init", &init)?;
        let res = room(&env, &code)?.fetch_with_request(inner).await?;
        if res.status_code() != 200 {
            return json_error(500, "couldn't create the game");
        }
        return Response::from_json(&serde_json::json!({ "code": code }));
    }
    if let Some(code) = path.strip_prefix("/ws/") {
        let code = code.to_ascii_uppercase();
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
            return json_error(404, "no such game");
        }
        return room(&env, &code)?.fetch_with_request(req).await;
    }
    json_error(404, "not found")
}

fn room(env: &Env, code: &str) -> Result<Stub> {
    env.durable_object("GAMES")?.id_from_name(code)?.get_stub()
}

fn json_error(status: u16, message: &str) -> Result<Response> {
    Ok(Response::from_json(&serde_json::json!({ "error": message }))?.with_status(status))
}

fn check_seats(seats: &[String]) -> std::result::Result<(), String> {
    let config = GameConfig::embedded();
    if !config.rules.starting_positions.contains_key(&seats.len()) {
        return Err(format!("{} players isn't supported", seats.len()));
    }
    if let Some(k) = seats
        .iter()
        .find(|k| *k != HUMAN && !BOT_KINDS.contains(&k.as_str()))
    {
        return Err(format!("seat kind {k} isn't available online"));
    }
    if !seats.iter().any(|k| k == HUMAN) {
        return Err("an online game needs a human seat".into());
    }
    Ok(())
}

/// The game in memory, rebuilt from storage after the object wakes.
struct Loaded {
    meta: Meta,
    game: Game,
    claims: Vec<Option<Claim>>,
}

#[durable_object]
pub struct GameRoom {
    state: State,
    loaded: RefCell<Option<Loaded>>,
}

impl DurableObject for GameRoom {
    fn new(state: State, _env: Env) -> Self {
        GameRoom {
            state,
            loaded: RefCell::new(None),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        if req.path() == "/init" {
            let meta: Meta = serde_json::from_str(&req.text().await?)?;
            let storage = self.state.storage();
            if storage.get::<Meta>("meta").await?.is_none() {
                let claims: Vec<Option<Claim>> = vec![None; meta.seats.len()];
                storage.put("claims", &claims).await?;
                storage.put("meta", &meta).await?;
            }
            return Response::ok("ok");
        }
        if req.headers().get("Upgrade")?.as_deref() != Some("websocket") {
            return json_error(426, "expected a WebSocket");
        }
        if !self.load().await? {
            return json_error(404, "no such game");
        }
        let pair = WebSocketPair::new()?;
        self.state.accept_web_socket(&pair.server);
        pair.server.serialize_attachment(Attachment {
            seat: None,
            joined: now(),
        })?;
        Response::from_websocket(pair.client)
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        let WebSocketIncomingMessage::String(text) = message else {
            return send(&ws, &error("expected text"));
        };
        if !self.load().await? {
            return send(&ws, &error("no such game"));
        }
        match serde_json::from_str::<ClientMsg>(&text) {
            Ok(ClientMsg::Join { name, seat, token }) => self.join(&ws, name, seat, token).await,
            Ok(ClientMsg::Action { index, action }) => self.action(&ws, index, action).await,
            Err(e) => send(&ws, &error(&format!("bad message: {e}"))),
        }
    }

    async fn websocket_close(
        &self,
        ws: WebSocket,
        _code: usize,
        _reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        self.closed(&ws).await
    }

    async fn websocket_error(&self, ws: WebSocket, _error: Error) -> Result<()> {
        self.closed(&ws).await
    }
}

fn error(message: &str) -> ServerMsg<'static> {
    ServerMsg::Error {
        message: message.to_string(),
    }
}

fn send(ws: &WebSocket, msg: &ServerMsg) -> Result<()> {
    ws.send_with_str(serde_json::to_string(msg)?)
}

fn attachment(ws: &WebSocket) -> Attachment {
    ws.deserialize_attachment()
        .ok()
        .flatten()
        .unwrap_or(Attachment {
            seat: None,
            joined: f64::MAX,
        })
}

impl GameRoom {
    async fn closed(&self, ws: &WebSocket) -> Result<()> {
        // The closing socket may still be listed; mark it gone first.
        ws.serialize_attachment(Attachment {
            seat: None,
            joined: f64::MAX,
        })?;
        if self.load().await? {
            self.broadcast_seats(Some(ws))?;
        }
        Ok(())
    }

    /// Loads the game into memory if it isn't there. False if this object
    /// was never set up (an unknown code).
    async fn load(&self) -> Result<bool> {
        if self.loaded.borrow().is_some() {
            return Ok(true);
        }
        let storage = self.state.storage();
        let Some(meta) = storage.get::<Meta>("meta").await? else {
            return Ok(false);
        };
        let claims: Vec<Option<Claim>> = storage
            .get("claims")
            .await?
            .unwrap_or_else(|| vec![None; meta.seats.len()]);
        // Moves are stored one per key, h0000000, h0000001, ... in order.
        let moves = storage
            .list_with_options(ListOptions::new().prefix("h"))
            .await?;
        let mut game = Game::new(
            Arc::new(GameConfig::embedded()),
            meta.seats.len(),
            meta.seed,
        )
        .map_err(|e| Error::RustError(e.to_string()))?;
        for value in moves.values() {
            let action: Action = serde_wasm_bindgen::from_value(value?)?;
            game.apply(action)
                .map_err(|e| Error::RustError(format!("stored move is illegal: {}", e.reason)))?;
        }
        // Another event may have loaded it while this one awaited storage.
        let mut slot = self.loaded.borrow_mut();
        if slot.is_none() {
            *slot = Some(Loaded { meta, game, claims });
        }
        Ok(true)
    }

    fn names(l: &Loaded) -> Vec<String> {
        l.claims
            .iter()
            .map(|c| c.as_ref().map(|c| c.name.clone()).unwrap_or_default())
            .collect()
    }

    async fn join(
        &self,
        ws: &WebSocket,
        name: String,
        seat: Option<usize>,
        token: Option<String>,
    ) -> Result<()> {
        let name: String = name.trim().chars().take(24).collect();
        let (you, claims) = {
            let mut guard = self.loaded.borrow_mut();
            let l = guard.as_mut().expect("loaded");
            let n = l.meta.seats.len();
            // A stored token gets its seat back.
            let held = token.as_deref().and_then(|t| {
                l.claims
                    .iter()
                    .position(|c| c.as_ref().is_some_and(|c| c.token == t))
            });
            let you = if let Some(s) = held {
                You {
                    seat: Some(s),
                    token: token.clone(),
                }
            } else if let Some(s) = seat {
                if s >= n || l.meta.seats[s] != HUMAN {
                    return send(ws, &error("that seat isn't a human seat"));
                }
                if l.claims[s].is_some() {
                    return send(ws, &error("that seat is taken"));
                }
                let token = random_hex();
                let name = if name.is_empty() {
                    format!("P{}", s + 1)
                } else {
                    name
                };
                l.claims[s] = Some(Claim {
                    name,
                    token: token.clone(),
                });
                You {
                    seat: Some(s),
                    token: Some(token),
                }
            } else {
                You {
                    seat: None,
                    token: None,
                }
            };
            (you, l.claims.clone())
        };
        if you.token.is_some() && token.as_deref() != you.token.as_deref() {
            self.state.storage().put("claims", &claims).await?;
        }
        let mut att = attachment(ws);
        att.seat = you.seat;
        ws.serialize_attachment(&att)?;
        {
            let guard = self.loaded.borrow();
            let l = guard.as_ref().expect("loaded");
            send(
                ws,
                &ServerMsg::Hello {
                    seats: &l.meta.seats,
                    seed: l.meta.seed,
                    history: l.game.history(),
                    names: Self::names(l),
                    you,
                },
            )?;
        }
        self.broadcast_seats(None)
    }

    /// The socket that plays the bots: the earliest joined one with a seat.
    fn bot_runner(&self, skip: Option<&WebSocket>) -> Option<(WebSocket, usize)> {
        self.state
            .get_websockets()
            .into_iter()
            .filter(|w| skip.is_none_or(|s| s != w))
            .filter_map(|w| {
                let a = attachment(&w);
                a.seat.map(|seat| (w, seat, a.joined))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(w, seat, _)| (w, seat))
    }

    fn broadcast_seats(&self, skip: Option<&WebSocket>) -> Result<()> {
        let sockets: Vec<WebSocket> = self
            .state
            .get_websockets()
            .into_iter()
            .filter(|w| skip.is_none_or(|s| s != w))
            .collect();
        let msg = {
            let guard = self.loaded.borrow();
            let l = guard.as_ref().expect("loaded");
            let mut connected = vec![false; l.meta.seats.len()];
            for w in &sockets {
                if let Some(s) = attachment(w).seat {
                    connected[s] = true;
                }
            }
            ServerMsg::Seats {
                names: Self::names(l),
                connected,
                bot_runner: self.bot_runner(skip).map(|(_, seat)| seat),
            }
        };
        let text = serde_json::to_string(&msg)?;
        for w in &sockets {
            // A socket that just went away can't be sent to; skip it.
            let _ = w.send_with_str(&text);
        }
        Ok(())
    }

    async fn action(&self, ws: &WebSocket, index: usize, action: Action) -> Result<()> {
        let Some(seat) = attachment(ws).seat else {
            return send(ws, &error("join a seat to play"));
        };
        let is_runner = self.bot_runner(None).is_some_and(|(w, _)| &w == ws);
        let result = {
            let mut guard = self.loaded.borrow_mut();
            let l = guard.as_mut().expect("loaded");
            let len = l.game.history().len();
            match l.game.current_player() {
                _ if index != len => Err(format!(
                    "out of date: move {index} sent, the game is at move {len}"
                )),
                None => Err("the game is over".to_string()),
                Some(p) => {
                    let bot_seat = l.meta.seats[p.0] != HUMAN;
                    if p.0 != seat && !(bot_seat && is_runner) {
                        Err("it isn't your turn".to_string())
                    } else {
                        l.game
                            .apply(action.clone())
                            .map(|_| len)
                            .map_err(|e| e.reason)
                    }
                }
            }
        };
        let index = match result {
            Ok(i) => i,
            Err(e) => return send(ws, &error(&e)),
        };
        // Broadcasts wait for this write to commit (output gate), so no
        // browser sees a move the object could lose.
        self.state
            .storage()
            .put(&format!("h{index:07}"), &action)
            .await?;
        let text = serde_json::to_string(&ServerMsg::Applied {
            index,
            action: &action,
        })?;
        for w in self.state.get_websockets() {
            let _ = w.send_with_str(&text);
        }
        Ok(())
    }
}
