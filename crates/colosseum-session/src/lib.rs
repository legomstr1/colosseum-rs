//! Game sessions and the JSON API the browser GUI talks to.
//!
//! The same code runs in two places: inside the axum server
//! (`colosseum-web`) and inside the browser as WebAssembly
//! (`colosseum-wasm`). Both pass requests to [`Api::route`], which takes a
//! method, a path and a JSON body and returns a status and a JSON body, so
//! the page can't tell which one it is talking to.
//!
//! Which bots exist and where seeds come from differ between the two, so
//! [`Api`] takes them as plain functions.

use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Game, GameConfig, GameEvent, Phase, PlayerId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub mod recover;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeatKind {
    Human,
    #[serde(rename = "live-input")]
    LiveInput,
    #[serde(rename = "tuned-1")]
    Tuned1,
    #[serde(rename = "tuned-2")]
    Tuned2,
    #[serde(rename = "int-event")]
    IntEvent,
    Search,
    House,
    Greedy,
    Random,
    Oracle,
    Maestro,
    Chameleon,
    Trader,
    Planner,
    Auctioneer,
    Showman,
    Rollout,
}

pub type Bot = Box<dyn Agent + Send>;

/// Builds the bot for a seat, or `None` for a human seat. Only called for
/// kinds listed in [`Api::kinds`].
pub type BotFactory = fn(SeatKind, u64) -> Option<Bot>;

#[derive(Clone, Debug, Serialize)]
pub struct LogEntry {
    /// Position in the whole log, so the page can tell what is new.
    pub n: usize,
    pub round: u32,
    /// "round", "phase" or "event".
    pub kind: &'static str,
    pub text: String,
}

fn new_game_entry(seed: u64) -> LogEntry {
    LogEntry {
        n: 0,
        round: 1,
        kind: "round",
        text: format!("New game, seed {seed}"),
    }
}

fn event_kind(e: &GameEvent) -> &'static str {
    match e {
        GameEvent::RoundStarted { .. } => "round",
        GameEvent::PhaseStarted { .. } => "phase",
        _ => "event",
    }
}

pub struct Session {
    pub game: Game,
    pub seats: Vec<SeatKind>,
    bots: Vec<Option<Bot>>,
    pub log: Vec<LogEntry>,
}

fn make_bots(factory: BotFactory, seats: &[SeatKind], seed: u64) -> Vec<Option<Bot>> {
    seats
        .iter()
        .enumerate()
        .map(|(i, kind)| match kind {
            SeatKind::Human => None,
            k => factory(*k, seed.wrapping_add(i as u64 + 1)),
        })
        .collect()
}

impl Session {
    /// Replays `history` from `seed`, rebuilding the log. Bots are new
    /// instances, so after a load they may choose differently than before.
    pub fn replay(
        config: Arc<GameConfig>,
        factory: BotFactory,
        seats: Vec<SeatKind>,
        seed: u64,
        history: &[Action],
    ) -> Result<Self, String> {
        let game = Game::new(config, seats.len(), seed).map_err(|e| e.to_string())?;
        let mut s = Session {
            game,
            bots: make_bots(factory, &seats, seed),
            seats,
            log: vec![new_game_entry(seed)],
        };
        for (i, a) in history.iter().enumerate() {
            let events = s
                .game
                .apply(a.clone())
                .map_err(|e| format!("action {i} ({a:?}) is illegal: {}", e.reason))?;
            s.record(&events);
        }
        Ok(s)
    }

    fn record(&mut self, events: &[GameEvent]) {
        for e in events {
            self.log.push(LogEntry {
                n: self.log.len(),
                round: self.game.round(),
                kind: event_kind(e),
                text: e.to_string(),
            });
        }
    }

    /// Applies one action and returns a frame for the page to animate: who
    /// acted, what they did, the log lines it added and the state after it.
    pub fn step(&mut self, action: Action) -> Result<Frame, String> {
        let actor = self.game.current_player().ok_or("the game is over")?;
        let text = self.game.describe_action(&action);
        let log_from = self.log.len();
        let events = self.game.apply(action).map_err(|e| e.reason)?;
        self.record(&events);
        let mut v = view_with("", self, None);
        // The page has the full log and legal moves from the final view.
        v.log = &[];
        v.legal = Vec::new();
        let state = serde_json::to_value(&v).map_err(|e| e.to_string())?;
        Ok(Frame {
            actor,
            action: text,
            events,
            log_from,
            log_to: self.log.len(),
            state,
        })
    }

    /// Lets bots act until a human seat must decide or the game ends.
    /// Returns a frame per bot action.
    pub fn run_bots(&mut self) -> Vec<Frame> {
        let mut frames = Vec::new();
        while let Some(p) = self.game.current_player() {
            let Some(bot) = self.bots[p.0].as_mut() else {
                break;
            };
            let legal = self.game.legal_actions();
            let action = bot.choose_action(&self.game, &legal);
            frames.push(self.step(action).expect("bots only pick legal actions"));
        }
        frames
    }

    pub fn saved(&self) -> SavedGame {
        SavedGame {
            seats: self.seats.clone(),
            seed: self.game.seed(),
            history: self.game.history().to_vec(),
        }
    }
}

/// One applied action, for the page to play back move by move.
#[derive(Serialize)]
pub struct Frame {
    actor: PlayerId,
    /// `describe_action` text, from before the action was applied.
    action: String,
    /// What happened, for animations (token flights, noble moves...).
    events: Vec<GameEvent>,
    /// The log entries `log_from..log_to` came from this action.
    log_from: usize,
    log_to: usize,
    /// The view after the action, without `log` or `legal`.
    state: Value,
}

#[derive(Serialize)]
struct LegalAction {
    action: Action,
    text: String,
}

/// Everything the page draws. Built fresh for every response.
#[derive(Serialize)]
pub struct View<'a> {
    id: &'a str,
    seed: u64,
    round: u32,
    num_rounds: u32,
    start_player: PlayerId,
    phase: &'a Phase,
    current_player: Option<PlayerId>,
    /// True when the player to act is a human, so `legal` is filled in.
    human_to_act: bool,
    /// The human to act may propose a trade.
    can_propose: bool,
    /// The human to act is answering an offer and may counter it.
    can_counter: bool,
    seats: &'a [SeatKind],
    players: &'a [colosseum_engine::PlayerState],
    stars_held: Vec<usize>,
    nobles: &'a [colosseum_engine::Noble],
    markets: &'a [Vec<colosseum_engine::Asset>],
    programs_for_sale: &'a [u8],
    supply: &'a colosseum_engine::Supply,
    stars: Vec<(colosseum_engine::Asset, Option<PlayerId>)>,
    bag_count: usize,
    legal: Vec<LegalAction>,
    log: &'a [LogEntry],
    game_over: bool,
    ranking: Vec<PlayerId>,
    /// Actions taken so far.
    history_len: usize,
    /// Set when this view is a past position of the game (read-only).
    replay: Option<ReplayInfo>,
    /// The actions this request applied, in order, for the page to animate.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    trail: Vec<Frame>,
}

/// Where a step-through view sits in the game it was taken from.
#[derive(Serialize)]
struct ReplayInfo {
    /// Actions applied to reach this position.
    step: usize,
    /// Actions in the whole game.
    total: usize,
    /// Steps where a phase starts, for jumping between phases.
    marks: Vec<Mark>,
    /// The action that led here and who took it.
    last: Option<(PlayerId, String)>,
    /// The action taken from this position, and who will take it.
    next: Option<(PlayerId, String)>,
}

#[derive(Serialize)]
struct Mark {
    step: usize,
    round: u32,
    phase: String,
}

fn phase_label(p: &Phase) -> String {
    p.kind()
        .map_or_else(|| "GameOver".to_string(), |k| format!("{k:?}"))
}

fn view_with<'a>(id: &'a str, s: &'a Session, replay: Option<ReplayInfo>) -> View<'a> {
    let g = &s.game;
    let current = g.current_player();
    let human_to_act = replay.is_none() && current.is_some_and(|p| s.seats[p.0] == SeatKind::Human);
    let legal = if human_to_act {
        g.legal_actions()
            .into_iter()
            .map(|a| LegalAction {
                text: g.describe_action(&a),
                action: a,
            })
            .collect()
    } else {
        Vec::new()
    };
    let (can_propose, can_counter) = match (g.phase(), current) {
        (Phase::Trading { offer, .. }, Some(_)) if human_to_act => {
            (offer.is_none(), offer.is_some())
        }
        _ => (false, false),
    };
    View {
        id,
        seed: g.seed(),
        round: g.round(),
        num_rounds: g.rules().num_rounds,
        start_player: g.start_player(),
        phase: g.phase(),
        current_player: current,
        human_to_act,
        can_propose,
        can_counter,
        seats: &s.seats,
        players: g.players(),
        stars_held: (0..g.num_players())
            .map(|i| g.stars_held(PlayerId(i)))
            .collect(),
        nobles: g.nobles(),
        markets: g.markets(),
        programs_for_sale: g.programs_for_sale(),
        supply: g.supply(),
        stars: g.stars().iter().map(|(a, h)| (*a, *h)).collect(),
        bag_count: g.bag().len() + g.green_pool().len(),
        legal,
        log: &s.log,
        game_over: g.is_over(),
        ranking: g.ranking(),
        history_len: g.history().len(),
        replay,
        trail: Vec::new(),
    }
}

/// Everything needed to rebuild a game: replaying `history` from `seed`.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedGame {
    pub seats: Vec<SeatKind>,
    pub seed: u64,
    pub history: Vec<Action>,
}

#[derive(Deserialize)]
struct NewGame {
    seats: Vec<SeatKind>,
    seed: Option<u64>,
}

#[derive(Deserialize)]
struct PostAction {
    action: Action,
}

#[derive(Deserialize)]
struct LoadGame {
    #[serde(flatten)]
    saved: SavedGame,
    /// Keep this id (so an old page URL works again). A new one by default.
    id: Option<String>,
}

#[derive(Deserialize)]
struct RecoverGame {
    seats: Vec<SeatKind>,
    seed: u64,
    /// The `text` of every log entry after the "New game" line.
    log: Vec<String>,
}

/// An API error: an HTTP status and a message.
pub type ApiError = (u16, String);
type ApiResult = Result<Value, ApiError>;

fn err(status: u16, message: impl Into<String>) -> ApiError {
    (status, message.into())
}

fn json(v: impl Serialize) -> ApiResult {
    serde_json::to_value(v).map_err(|e| err(500, e.to_string()))
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(|e| err(400, format!("bad request: {e}")))
}

/// Every game in memory, plus what the host provides.
pub struct Api {
    pub config: Arc<GameConfig>,
    /// Seat kinds offered on the setup screen ("human" included).
    pub kinds: Vec<SeatKind>,
    pub bots: BotFactory,
    /// A seed for a game created without one.
    pub fresh_seed: fn() -> u64,
    games: HashMap<String, Session>,
}

impl Api {
    pub fn new(
        config: GameConfig,
        kinds: Vec<SeatKind>,
        bots: BotFactory,
        fresh_seed: fn() -> u64,
    ) -> Self {
        Api {
            config: Arc::new(config),
            kinds,
            bots,
            fresh_seed,
            games: HashMap::new(),
        }
    }

    pub fn game(&self, id: &str) -> Option<&Session> {
        self.games.get(id)
    }

    /// Handles one request. `path` may carry a query string. Returns the
    /// HTTP status and the JSON response body.
    pub fn route(&mut self, method: &str, path: &str, body: &str) -> (u16, String) {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        let result = match (method, parts.as_slice()) {
            ("GET", ["api", "config"]) => json(self.config.as_ref()),
            ("GET", ["api", "seats"]) => json(&self.kinds),
            ("POST", ["api", "games"]) => parse(body).and_then(|r| self.create(r)),
            ("POST", ["api", "games", "load"]) => parse(body).and_then(|r| self.load(r)),
            ("POST", ["api", "games", "recover"]) => parse(body).and_then(|r| self.recover(r)),
            ("GET", ["api", "games", id]) => self.get(id),
            ("POST", ["api", "games", id, "actions"]) => {
                parse(body).and_then(|r| self.action(id, r))
            }
            ("GET", ["api", "games", id, "save"]) => self.save(id),
            ("POST", ["api", "games", id, "undo"]) => self.undo(id),
            ("GET", ["api", "games", id, "step"]) => {
                let step = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("step="))
                    .and_then(|v| v.parse().ok());
                match step {
                    Some(step) => self.step(id, step),
                    None => Err(err(400, "missing ?step=N")),
                }
            }
            _ => Err(err(404, "no such endpoint")),
        };
        match result {
            Ok(v) => (200, v.to_string()),
            Err((status, message)) => (status, serde_json::json!({ "error": message }).to_string()),
        }
    }

    fn session(&self, id: &str) -> Result<&Session, ApiError> {
        self.games.get(id).ok_or_else(|| err(404, "no such game"))
    }

    fn session_mut(&mut self, id: &str) -> Result<&mut Session, ApiError> {
        self.games
            .get_mut(id)
            .ok_or_else(|| err(404, "no such game"))
    }

    fn check_kinds(&self, seats: &[SeatKind]) -> Result<(), ApiError> {
        match seats.iter().find(|k| !self.kinds.contains(k)) {
            Some(k) => Err(err(400, format!("seat kind {k:?} isn't available here"))),
            None => Ok(()),
        }
    }

    fn view_trail(id: &str, s: &Session, trail: Vec<Frame>) -> ApiResult {
        let mut v = view_with(id, s, None);
        v.trail = trail;
        json(v)
    }

    fn create(&mut self, req: NewGame) -> ApiResult {
        self.check_kinds(&req.seats)?;
        let seed = req.seed.unwrap_or_else(self.fresh_seed);
        let mut session = Session::replay(self.config.clone(), self.bots, req.seats, seed, &[])
            .map_err(|e| err(400, e))?;
        let trail = session.run_bots();
        let id = format!(
            "{:x}",
            seed ^ (self.games.len() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        );
        let response = Self::view_trail(&id, &session, trail);
        self.games.insert(id, session);
        response
    }

    fn get(&self, id: &str) -> ApiResult {
        json(view_with(id, self.session(id)?, None))
    }

    fn action(&mut self, id: &str, req: PostAction) -> ApiResult {
        let s = self.session_mut(id)?;
        let Some(p) = s.game.current_player() else {
            return Err(err(409, "the game is over"));
        };
        if s.seats[p.0] != SeatKind::Human {
            return Err(err(409, "it's a bot's turn"));
        }
        let first = s.step(req.action).map_err(|e| err(422, e))?;
        let mut trail = vec![first];
        trail.extend(s.run_bots());
        Self::view_trail(id, s, trail)
    }

    fn save(&self, id: &str) -> ApiResult {
        json(self.session(id)?.saved())
    }

    fn insert(&mut self, id: Option<String>, mut session: Session) -> ApiResult {
        session.run_bots();
        let n = self.games.len() as u64 + 1;
        let id = id.unwrap_or_else(|| {
            format!(
                "{:x}",
                session.game.seed() ^ n.wrapping_mul(0xbf58_476d_1ce4_e5b9)
            )
        });
        let response = json(view_with(&id, &session, None));
        self.games.insert(id, session);
        response
    }

    fn load(&mut self, req: LoadGame) -> ApiResult {
        let SavedGame {
            seats,
            seed,
            history,
        } = req.saved;
        self.check_kinds(&seats)?;
        let session = Session::replay(self.config.clone(), self.bots, seats, seed, &history)
            .map_err(|e| err(422, e))?;
        self.insert(req.id, session)
    }

    /// Rebuilds a game's history from its seed and log text, for games on a
    /// server that had no save endpoint. Returns a `SavedGame`.
    fn recover(&self, req: RecoverGame) -> ApiResult {
        let game = Game::new(self.config.clone(), req.seats.len(), req.seed)
            .map_err(|e| err(400, e.to_string()))?;
        let human: Vec<bool> = req.seats.iter().map(|k| *k == SeatKind::Human).collect();
        let r = recover::recover(game, &req.log, &human).map_err(|e| err(422, e))?;
        json(SavedGame {
            seats: req.seats,
            seed: req.seed,
            history: r.history,
        })
    }

    /// The game as it stood after its first `step` actions, read-only, for
    /// stepping through a game. Bots aren't consulted: the history is replayed.
    fn step(&self, id: &str, step: usize) -> ApiResult {
        let s = self.session(id)?;
        let (seats, seed, history) = (s.seats.clone(), s.game.seed(), s.game.history());
        let step = step.min(history.len());
        let mut g = Game::new(self.config.clone(), seats.len(), seed)
            .map_err(|e| err(500, e.to_string()))?;
        let mut marks = vec![Mark {
            step: 0,
            round: g.round(),
            phase: phase_label(g.phase()),
        }];
        let mut at_step = None;
        let mut last = None;
        let mut next = None;
        let mut log = vec![new_game_entry(seed)];
        for (i, a) in history.iter().enumerate() {
            if i == step {
                at_step = Some((g.clone(), std::mem::take(&mut log)));
            }
            let who = g.current_player().unwrap_or(PlayerId(0));
            let text = g.describe_action(a);
            if i + 1 == step {
                last = Some((who, text.clone()));
            }
            if i == step {
                next = Some((who, text));
            }
            let before = (g.round(), phase_label(g.phase()));
            let events = g.apply(a.clone()).map_err(|e| err(500, e.to_string()))?;
            if i < step {
                for e in &events {
                    log.push(LogEntry {
                        n: log.len(),
                        round: g.round(),
                        kind: event_kind(e),
                        text: e.to_string(),
                    });
                }
            }
            let after = (g.round(), phase_label(g.phase()));
            if after != before {
                marks.push(Mark {
                    step: i + 1,
                    round: after.0,
                    phase: after.1,
                });
            }
        }
        let total = history.len();
        let (game, log) = at_step.unwrap_or((g, log));
        let session = Session {
            game,
            bots: seats.iter().map(|_| None).collect(),
            seats,
            log,
        };
        let info = ReplayInfo {
            step,
            total,
            marks,
            last,
            next,
        };
        json(view_with(id, &session, Some(info)))
    }

    /// Takes back the last human action and everything after it.
    fn undo(&mut self, id: &str) -> ApiResult {
        let config = self.config.clone();
        let factory = self.bots;
        let s = self.session_mut(id)?;
        // Who made each action: replay and ask before each one.
        let history = s.game.history().to_vec();
        let mut g = Game::new(config.clone(), s.seats.len(), s.game.seed())
            .map_err(|e| err(500, e.to_string()))?;
        let mut last_human = None;
        for (i, a) in history.iter().enumerate() {
            if g.current_player()
                .is_some_and(|p| s.seats[p.0] == SeatKind::Human)
            {
                last_human = Some(i);
            }
            g.apply(a.clone()).expect("history replays");
        }
        let Some(cut) = last_human else {
            return Err(err(409, "nothing to undo"));
        };
        *s = Session::replay(
            config,
            factory,
            s.seats.clone(),
            s.game.seed(),
            &history[..cut],
        )
        .map_err(|e| err(500, e))?;
        json(view_with(id, s, None))
    }
}
