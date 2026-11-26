//! The GUI's JSON API, running inside the browser.
//!
//! `static/app.js` normally `fetch`es `/api/...` from `colosseum-web`. When
//! this module is loaded it calls [`Local::request`] instead, which runs the
//! same [`colosseum_session::Api`] code, so no server is involved. Only the
//! live-input bot is built in, to keep the download small.

use colosseum_engine::GameConfig;
use colosseum_session::{Api, Bot, SeatKind};
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

fn fresh_seed() -> u64 {
    let r = (js_sys::Math::random() * (1u64 << 53) as f64) as u64;
    r ^ (js_sys::Date::now() as u64).rotate_left(32)
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

#[wasm_bindgen(getter_with_clone)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

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
