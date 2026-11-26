//! The Cloudflare Worker for online play (see `docs/ONLINE.md`).
//!
//! Static files (the page and the WebAssembly engine, built into `dist/`)
//! are served by Workers Static Assets and never reach this code. The
//! Worker handles:
//! - `POST /api/login {password}`: checks the shared password (secret
//!   `SITE_PASSWORD`) and sets a signed cookie. Everything below needs it.
//! - `POST /api/online/games {seats}`: makes a game code and sets up its
//!   Durable Object.
//! - `GET /api/online/games/{code}`: whether that game exists.
//! - `GET /ws/{code}`: a WebSocket to that game's Durable Object.
//!
//! Each game is one [`GameRoom`]. It keeps the seed, the seats, who holds
//! which seat, and the ordered moves. It checks every move with the engine
//! and relays accepted ones to every browser. Browsers run the engine
//! themselves, and the one on bot duty sends the bots' moves.

use colosseum_engine::{Action, Game, GameConfig};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
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
        /// Human seats a bot is playing until their player comes back.
        stand_in: Vec<bool>,
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

#[derive(Deserialize)]
struct Login {
    password: String,
}

// ---------- the shared password ----------

type HmacSha256 = Hmac<Sha256>;
const COOKIE: &str = "imp_auth";
const COOKIE_DAYS: u64 = 30;

/// The key cookies are signed with. It comes from the password, so changing
/// the password signs everyone out.
fn cookie_key(password: &str) -> [u8; 32] {
    Sha256::digest(format!("colosseum-auth-v1:{password}").as_bytes()).into()
}

fn sign(key: &[u8], msg: &str) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(msg.as_bytes());
    mac
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Constant-time password check: compare HMACs of both, never the strings.
fn password_matches(given: &str, real: &str) -> bool {
    let key = b"colosseum-password-compare";
    let expected = sign(key, real).finalize().into_bytes();
    sign(key, given).verify_slice(&expected).is_ok()
}

/// The cookie value: `<expiry ms>.<hex HMAC of the expiry>`.
fn make_cookie(password: &str, expires_ms: u64) -> String {
    let exp = expires_ms.to_string();
    let tag = sign(&cookie_key(password), &exp).finalize().into_bytes();
    format!("{exp}.{}", hex(&tag))
}

fn cookie_valid(password: &str, value: &str, now_ms: u64) -> bool {
    let Some((exp, tag)) = value.split_once('.') else {
        return false;
    };
    let (Ok(expires), Some(tag)) = (exp.parse::<u64>(), unhex(tag)) else {
        return false;
    };
    expires > now_ms && sign(&cookie_key(password), exp).verify_slice(&tag).is_ok()
}

fn site_password(env: &Env) -> Option<String> {
    env.secret("SITE_PASSWORD")
        .ok()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn request_cookie(req: &Request, name: &str) -> Option<String> {
    let header = req.headers().get("Cookie").ok()??;
    header.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

fn authed(req: &Request, password: &str) -> bool {
    request_cookie(req, COOKIE).is_some_and(|v| cookie_valid(password, &v, now() as u64))
}

async fn login(mut req: Request, env: &Env, password: &str) -> Result<Response> {
    // At most 5 tries a minute per address, counted in one LoginLimiter.
    let ip = req
        .headers()
        .get("CF-Connecting-IP")?
        .unwrap_or_else(|| "local".into());
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_body(Some(ip.into()));
    let check = Request::new_with_init("https://limiter/try", &init)?;
    let limiter = env
        .durable_object("LIMITER")?
        .id_from_name("login")?
        .get_stub()?;
    if limiter.fetch_with_request(check).await?.status_code() != 200 {
        return json_error(429, "too many tries; wait a minute");
    }
    let body: Login = match serde_json::from_str(&req.text().await?) {
        Ok(b) => b,
        Err(_) => return json_error(400, "bad request"),
    };
    if !password_matches(&body.password, password) {
        return json_error(401, "wrong password");
    }
    let max_age = COOKIE_DAYS * 24 * 3600;
    let value = make_cookie(password, now() as u64 + max_age * 1000);
    let secure = if req.url()?.scheme() == "https" {
        "; Secure"
    } else {
        ""
    };
    let mut res = Response::from_json(&serde_json::json!({ "ok": true }))?;
    res.headers_mut().set(
        "Set-Cookie",
        &format!("{COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}"),
    )?;
    Ok(res)
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // Fail closed: without a password nobody gets in.
    let Some(password) = site_password(&env) else {
        return json_error(503, "the server has no SITE_PASSWORD set");
    };
    if req.method() == Method::Post && req.path() == "/api/login" {
        return login(req, &env, &password).await;
    }
    if !authed(&req, &password) {
        return json_error(401, "log in first");
    }
    games(req, env).await
}

/// A game code from a path segment, if it looks like one.
fn parse_code(s: &str) -> Option<String> {
    let code = s.to_ascii_uppercase();
    (code.len() == 6 && code.chars().all(|c| c.is_ascii_alphanumeric())).then_some(code)
}

async fn games(mut req: Request, env: Env) -> Result<Response> {
    let path = req.path();
    if let Some(code) = path.strip_prefix("/api/online/games/") {
        let Some(code) = parse_code(code) else {
            return json_error(404, "no such game");
        };
        let ask = Request::new("https://room/exists", Method::Get)?;
        return room(&env, &code)?.fetch_with_request(ask).await;
    }
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
        let Some(code) = parse_code(code) else {
            return json_error(404, "no such game");
        };
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
    stand_in: Vec<bool>,
}

impl Loaded {
    /// Whether the seat's moves come from the bot runner.
    fn bot_plays(&self, seat: usize) -> bool {
        self.meta.seats[seat] != HUMAN || self.stand_in[seat]
    }
}

#[durable_object]
pub struct GameRoom {
    state: State,
    loaded: RefCell<Option<Loaded>>,
    /// How long the seat to act may sit empty before a bot plays it.
    stand_in_ms: i64,
}

impl DurableObject for GameRoom {
    fn new(state: State, env: Env) -> Self {
        let secs = env
            .var("STANDIN_SECONDS")
            .ok()
            .and_then(|v| v.to_string().parse::<i64>().ok())
            .unwrap_or(120);
        GameRoom {
            state,
            loaded: RefCell::new(None),
            stand_in_ms: secs.max(1) * 1000,
        }
    }

    async fn alarm(&self) -> Result<Response> {
        if !self.load().await? {
            return Response::ok("gone");
        }
        if let Some(p) = self.waiting_seat(None) {
            let stand_in = {
                let mut guard = self.loaded.borrow_mut();
                let l = guard.as_mut().expect("loaded");
                l.stand_in[p] = true;
                l.stand_in.clone()
            };
            self.state.storage().put("stand_in", &stand_in).await?;
            self.broadcast_seats(None)?;
        }
        Response::ok("ok")
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        if req.path() == "/exists" {
            return if self.load().await? {
                Response::from_json(&serde_json::json!({ "ok": true }))
            } else {
                json_error(404, "no such game")
            };
        }
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
            self.update_alarm(Some(ws)).await?;
        }
        Ok(())
    }

    /// Which seats have a connected socket.
    fn connected(&self, n: usize, skip: Option<&WebSocket>) -> Vec<bool> {
        let mut connected = vec![false; n];
        for w in self.state.get_websockets() {
            if skip.is_some_and(|s| s == &w) {
                continue;
            }
            if let Some(s) = attachment(&w).seat {
                connected[s] = true;
            }
        }
        connected
    }

    /// The seat to act, if it's a human seat with nobody connected and no
    /// bot standing in yet.
    fn waiting_seat(&self, skip: Option<&WebSocket>) -> Option<usize> {
        let guard = self.loaded.borrow();
        let l = guard.as_ref()?;
        let p = l.game.current_player()?.0;
        let connected = self.connected(l.meta.seats.len(), skip);
        (!l.bot_plays(p) && !connected[p]).then_some(p)
    }

    /// Starts the stand-in countdown when the seat to act sits empty, and
    /// cancels it otherwise, so an old countdown never fires early.
    async fn update_alarm(&self, skip: Option<&WebSocket>) -> Result<()> {
        let storage = self.state.storage();
        if self.waiting_seat(skip).is_some() {
            if storage.get_alarm().await?.is_none() {
                storage.set_alarm(self.stand_in_ms).await?;
            }
        } else {
            storage.delete_alarm().await?;
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
        let stand_in: Vec<bool> = storage
            .get("stand_in")
            .await?
            .unwrap_or_else(|| vec![false; meta.seats.len()]);
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
            *slot = Some(Loaded {
                meta,
                game,
                claims,
                stand_in,
            });
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
        let n = self
            .loaded
            .borrow()
            .as_ref()
            .expect("loaded")
            .meta
            .seats
            .len();
        // A seat can be taken over while nobody holds it (this socket aside).
        let connected = self.connected(n, Some(ws));
        let (you, claims, stand_in) = {
            let mut guard = self.loaded.borrow_mut();
            let l = guard.as_mut().expect("loaded");
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
                if connected[s] {
                    return send(ws, &error("that seat is taken"));
                }
                // A new token: whoever held this seat before loses it.
                let token = random_hex();
                let name = match (&l.claims[s], name.is_empty()) {
                    (_, false) => name,
                    (Some(old), true) => old.name.clone(),
                    (None, true) => format!("P{}", s + 1),
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
            // Its player is back: the bot stands down.
            let mut stand_in = None;
            if let Some(s) = you.seat.filter(|&s| l.stand_in[s]) {
                l.stand_in[s] = false;
                stand_in = Some(l.stand_in.clone());
            }
            (you, l.claims.clone(), stand_in)
        };
        if you.token.is_some() && token.as_deref() != you.token.as_deref() {
            self.state.storage().put("claims", &claims).await?;
        }
        if let Some(stand_in) = stand_in {
            self.state.storage().put("stand_in", &stand_in).await?;
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
        self.broadcast_seats(None)?;
        self.update_alarm(None).await
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
            ServerMsg::Seats {
                names: Self::names(l),
                connected: self.connected(l.meta.seats.len(), skip),
                bot_runner: self.bot_runner(skip).map(|(_, seat)| seat),
                stand_in: l.stand_in.clone(),
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
                    let bot_seat = l.bot_plays(p.0);
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
        self.update_alarm(None).await
    }
}

/// Counts login attempts per address, in memory. One instance serves every
/// login, so the count holds across the Worker's isolates. Losing it when
/// the object is evicted only resets the count.
#[durable_object]
pub struct LoginLimiter {
    tries: RefCell<HashMap<String, Vec<f64>>>,
}

const LOGIN_TRIES: usize = 5;
const LOGIN_WINDOW_MS: f64 = 60_000.0;

impl DurableObject for LoginLimiter {
    fn new(_state: State, _env: Env) -> Self {
        LoginLimiter {
            tries: RefCell::new(HashMap::new()),
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        let ip = req.text().await?;
        let t = now();
        let mut tries = self.tries.borrow_mut();
        tries.retain(|_, v| {
            v.retain(|&x| t - x < LOGIN_WINDOW_MS);
            !v.is_empty()
        });
        let mine = tries.entry(ip).or_default();
        if mine.len() >= LOGIN_TRIES {
            return json_error(429, "too many tries");
        }
        mine.push(t);
        Response::ok("ok")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies() {
        let c = make_cookie("open sesame", 5_000);
        assert!(cookie_valid("open sesame", &c, 4_000));
        assert!(!cookie_valid("open sesame", &c, 6_000), "expired");
        assert!(!cookie_valid("new password", &c, 4_000), "password changed");
        let (exp, tag) = c.split_once('.').unwrap();
        assert!(
            !cookie_valid("open sesame", &format!("9{exp}.{tag}"), 4_000),
            "expiry edited"
        );
        assert!(!cookie_valid("open sesame", "garbage", 4_000));
        assert!(password_matches("open sesame", "open sesame"));
        assert!(!password_matches("open sesamE", "open sesame"));
    }
}
