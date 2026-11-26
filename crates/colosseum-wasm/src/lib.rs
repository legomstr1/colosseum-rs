//! The GUI's JSON API, running inside the browser.
//!
//! `static/app.js` normally `fetch`es `/api/...` from `colosseum-web`. When
//! this module is loaded it calls [`Local::request`] instead, which runs the
//! same [`colosseum_session::Api`] code, so no server is involved. Only the
//! live-input bot is built in, to keep the download small.

use colosseum_engine::{Action, GameConfig};
use colosseum_session::{Api, Bot, Online, SeatKind, Session};
use std::sync::Arc;
use wasm_bindgen::prelude::*;

fn make_bot(kind: SeatKind, seed: u64) -> Option<Bot> {
    match kind {
        // Default config: no search, one thread.
        SeatKind::LiveInput => Some(Box::new(colosseum_bot_live_input::LiveInputAgent::new(
            seed,
        ))),
        _ => None,
    }
}

/// At most 53 bits, so the seed survives JavaScript numbers (the page
/// parses views and saves with `JSON.parse`).
fn fresh_seed() -> u64 {
    let r = (js_sys::Math::random() * (1u64 << 53) as f64) as u64;
    (r ^ js_sys::Date::now() as u64) & ((1 << 53) - 1)
}

/// One API instance holding every game played in this page.
#[wasm_bindgen]
pub struct Local {
    api: Api,
}

#[wasm_bindgen]
impl Local {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Local {
        let kinds = vec![SeatKind::Human, SeatKind::LiveInput];
        Local {
            api: Api::new(GameConfig::embedded(), kinds, make_bot, fresh_seed),
        }
    }

    /// Handles one request like the HTTP server would. Returns
    /// `{status, body}` where `body` is JSON text.
    pub fn request(&mut self, method: &str, path: &str, body: &str) -> Reply {
        let (status, body) = self.api.route(method, path, body);
        Reply { status, body }
    }
}

impl Default for Local {
    fn default() -> Self {
        Self::new()
    }
}

/// One online game as this browser sees it (see `web/online.js`). The
/// server decides the order of moves; this applies them as they arrive.
#[wasm_bindgen]
pub struct OnlineClient {
    id: String,
    session: Session,
}

#[wasm_bindgen]
impl OnlineClient {
    /// `seats` and `history` are JSON, `names` a JSON list of strings,
    /// `me` this browser's seat or -1 for a spectator.
    #[wasm_bindgen(constructor)]
    pub fn new(
        id: &str,
        seats: &str,
        seed: f64,
        history: &str,
        names: &str,
        me: i32,
    ) -> Result<OnlineClient, String> {
        let seats: Vec<SeatKind> = serde_json::from_str(seats).map_err(|e| e.to_string())?;
        let history: Vec<Action> = serde_json::from_str(history).map_err(|e| e.to_string())?;
        let names: Vec<String> = serde_json::from_str(names).map_err(|e| e.to_string())?;
        let mut session = Session::replay(
            Arc::new(GameConfig::embedded()),
            make_bot,
            seats,
            seed as u64,
            &history,
        )?;
        session.online = Some(Online {
            me: usize::try_from(me).ok(),
            names,
        });
        Ok(OnlineClient {
            id: id.to_string(),
            session,
        })
    }

    /// Moves applied so far; the next move's index.
    pub fn history_len(&self) -> usize {
        self.session.game.history().len()
    }

    /// The seat to act, or -1 when the game is over.
    pub fn current_player(&self) -> i32 {
        self.session
            .game
            .current_player()
            .map_or(-1, |p| p.0 as i32)
    }

    /// Whether the seat to act is played by a bot.
    pub fn bot_to_act(&self) -> bool {
        self.session
            .game
            .current_player()
            .is_some_and(|p| self.session.seats[p.0] != SeatKind::Human)
    }

    pub fn set_names(&mut self, names: &str) -> Result<(), String> {
        let names: Vec<String> = serde_json::from_str(names).map_err(|e| e.to_string())?;
        if let Some(o) = self.session.online.as_mut() {
            o.names = names;
        }
        Ok(())
    }

    /// Applies a move from the server. Returns the view after it, with a
    /// one-frame `trail`, as JSON.
    pub fn apply(&mut self, action: &str) -> Result<String, String> {
        let action: Action = serde_json::from_str(action).map_err(|e| e.to_string())?;
        Ok(self.session.apply_json(&self.id, action)?.to_string())
    }

    pub fn view(&self) -> String {
        self.session.view_json(&self.id).to_string()
    }

    /// `{seats, seed, history}` JSON.
    pub fn save(&self) -> String {
        serde_json::to_string(&self.session.saved()).expect("saves serialize")
    }

    /// The bot's move for the seat to act, as JSON, if a bot is to act.
    pub fn bot_action(&mut self) -> Option<String> {
        if !self.bot_to_act() {
            return None;
        }
        self.session
            .bot_action()
            .map(|a| serde_json::to_string(&a).expect("actions serialize"))
    }
}

#[wasm_bindgen(getter_with_clone)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two browsers see the same game; only the seat to act gets moves,
    /// and the bot seat's move comes from `bot_action`.
    #[test]
    fn online_clients_agree() {
        let seats = r#"["human","human","live-input"]"#;
        let mut a = OnlineClient::new("g", seats, 11.0, "[]", r#"["Ann","Bo",""]"#, 0).unwrap();
        let mut b = OnlineClient::new("g", seats, 11.0, "[]", r#"["Ann","Bo",""]"#, 1).unwrap();
        for _ in 0..400 {
            let cur = a.current_player();
            if cur < 0 {
                break;
            }
            let action = if a.bot_to_act() {
                a.bot_action().unwrap()
            } else {
                let mover = if cur == 0 { &a } else { &b };
                let v: serde_json::Value = serde_json::from_str(&mover.view()).unwrap();
                assert_eq!(v["human_to_act"], true);
                let other = if cur == 0 { &b } else { &a };
                let w: serde_json::Value = serde_json::from_str(&other.view()).unwrap();
                assert_eq!(w["human_to_act"], false, "only the seat to act gets moves");
                serde_json::to_string(&v["legal"][0]["action"]).unwrap()
            };
            a.apply(&action).unwrap();
            b.apply(&action).unwrap();
        }
        assert_eq!(a.save(), b.save());
        let v: serde_json::Value = serde_json::from_str(&a.view()).unwrap();
        assert_eq!(v["online"]["me"], 0);
        assert_eq!(v["online"]["names"][1], "Bo");
    }

    /// A full game of live-input bots through the API, the way the page
    /// drives it (native build; the same code runs in the browser).
    #[test]
    fn bot_game_through_the_api() {
        let mut api = Api::new(
            GameConfig::embedded(),
            vec![SeatKind::Human, SeatKind::LiveInput],
            make_bot,
            || 7,
        );
        let (status, body) = api.route(
            "POST",
            "/api/games",
            r#"{"seats":["live-input","live-input","live-input"],"seed":3}"#,
        );
        assert_eq!(status, 200, "{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["game_over"], true);
        let (status, _) = api.route(
            "POST",
            "/api/games",
            r#"{"seats":["human","house","house"]}"#,
        );
        assert_eq!(status, 400);
    }
}
