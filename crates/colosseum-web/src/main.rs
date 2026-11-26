//! `colosseum-web`: serves a browser UI and a small JSON API over the engine.
//!
//! The server owns every game. A browser creates a game, reads its view and
//! posts actions; bots move on the server until a human seat is to act. Seats
//! are part of each session so that remote players can join later.

use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::{Action, Game, GameConfig, GameEvent, Phase, PlayerId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

mod recover;

const INDEX_HTML: &str = include_str!("../static/index.html");
const APP_JS: &str = include_str!("../static/app.js");
const ICONS_JS: &str = include_str!("../static/icons.js");
const STYLE_CSS: &str = include_str!("../static/style.css");
// The phone layout at /m: its own page and script, the shared icons, and a
// stylesheet loaded on top of the desktop one.
const M_INDEX_HTML: &str = include_str!("../static/mobile/index.html");
const M_APP_JS: &str = include_str!("../static/mobile/app.js");
const M_STYLE_CSS: &str = include_str!("../static/mobile/style.css");

#[derive(Parser)]
#[command(name = "colosseum-web", about = "Play Colosseum in the browser")]
struct Cli {
    /// Port to listen on.
    #[arg(long, default_value_t = 8080)]
    port: u16,
    /// Address to bind. Use 0.0.0.0 to accept other machines.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Load game_config.yml and events.yml from this directory instead of
    /// the copies built into the engine.
    #[arg(long)]
    data: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SeatKind {
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

#[derive(Clone, Debug, Serialize)]
struct LogEntry {
    /// Position in the whole log, so the page can tell what is new.
    n: usize,
    round: u32,
    /// "round", "phase" or "event".
    kind: &'static str,
    text: String,
}

struct Session {
    game: Game,
    seats: Vec<SeatKind>,
    bots: Vec<Option<Box<dyn Agent + Send>>>,
    log: Vec<LogEntry>,
}

/// One bot per non-human seat, seeded from the game seed.
fn make_bots(seats: &[SeatKind], seed: u64) -> Vec<Option<Box<dyn Agent + Send>>> {
    seats
        .iter()
        .enumerate()
        .map(|(i, kind)| -> Option<Box<dyn Agent + Send>> {
            let bot_seed = seed.wrapping_add(i as u64 + 1);
            match kind {
                SeatKind::Human => None,
                SeatKind::Search => {
                    // Results don't depend on the thread count, only the speed.
                    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
                    let cfg = colosseum_bot_search::Config { threads, ..Default::default() };
                    Some(Box::new(colosseum_bot_search::SearchAgent::with_config(bot_seed, cfg)))
                }
                SeatKind::LiveInput => {
                    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
                    let cfg = colosseum_bot_live_input::SearchConfig { threads, ..Default::default() };
                    Some(Box::new(colosseum_bot_live_input::LiveInputAgent::with_config(bot_seed, Default::default(), cfg)))
                }
                SeatKind::Tuned1 => Some(Box::new(colosseum_bot_tuned_1::Tuned1Agent::new(bot_seed))),
                SeatKind::Tuned2 => Some(Box::new(colosseum_bot_tuned_2::Tuned2Agent::new(bot_seed))),
                SeatKind::IntEvent => Some(Box::new(colosseum_bot_int_event::IntEventAgent::new(bot_seed))),
                SeatKind::House => Some(Box::new(colosseum_bot_house::HouseAgent::new(bot_seed))),
                SeatKind::Greedy => Some(Box::new(GreedyAgent::new(bot_seed))),
                SeatKind::Random => Some(Box::new(RandomAgent::new(bot_seed))),
                SeatKind::Oracle => Some(Box::new(colosseum_bot_oracle::OracleAgent::new(bot_seed))),
                SeatKind::Maestro => Some(Box::new(colosseum_bot_maestro::MaestroAgent::new(bot_seed))),
                SeatKind::Chameleon => Some(Box::new(colosseum_bot_chameleon::ChameleonAgent::new(bot_seed))),
                SeatKind::Trader => Some(Box::new(colosseum_bot_trader::TraderAgent::new(bot_seed))),
                SeatKind::Planner => Some(Box::new(colosseum_bot_planner::PlannerAgent::new(bot_seed))),
                SeatKind::Auctioneer => Some(Box::new(colosseum_bot_auctioneer::AuctioneerAgent::new(bot_seed))),
                SeatKind::Showman => Some(Box::new(colosseum_bot_showman::ShowmanAgent::new(bot_seed))),
                SeatKind::Rollout => Some(Box::new(colosseum_bot_rollout::RolloutAgent::new(bot_seed))),
            }
        })
        .collect()
}

impl Session {
    /// Replays `history` from `seed`, rebuilding the log. Bots are new
    /// instances, so after a load they may choose differently than before.
    fn replay(config: Arc<GameConfig>, seats: Vec<SeatKind>, seed: u64, history: &[Action]) -> Result<Self, String> {
        let game = Game::new(config, seats.len(), seed).map_err(|e| e.to_string())?;
        let mut s = Session {
            game,
            bots: make_bots(&seats, seed),
            seats,
            log: Vec::new(),
        };
        s.log.push(LogEntry {
            n: 0,
            round: 1,
            kind: "round",
            text: format!("New game, seed {seed}"),
        });
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
            let kind = match e {
                GameEvent::RoundStarted { .. } => "round",
                GameEvent::PhaseStarted { .. } => "phase",
                _ => "event",
            };
            self.log.push(LogEntry {
                n: self.log.len(),
                round: self.game.round(),
                kind,
                text: e.to_string(),
            });
        }
    }

    /// Applies one action and returns a frame for the page to animate: who
    /// acted, what they did, the log lines it added and the state after it.
    fn step(&mut self, action: Action) -> Result<Frame, String> {
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
    fn run_bots(&mut self) -> Vec<Frame> {
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
}

/// One applied action, for the page to play back move by move.
#[derive(Serialize)]
struct Frame {
    actor: PlayerId,
    /// `describe_action` text, from before the action was applied.
    action: String,
    /// What happened, for animations (token flights, noble moves...).
    events: Vec<GameEvent>,
    /// The log entries `log_from..log_to` came from this action.
    log_from: usize,
    log_to: usize,
    /// The view after the action, without `log` or `legal`.
    state: serde_json::Value,
}

struct Server {
    config: Arc<GameConfig>,
    games: Mutex<HashMap<String, Session>>,
}

type Shared = Arc<Server>;

#[derive(Deserialize)]
struct NewGame {
    seats: Vec<SeatKind>,
    seed: Option<u64>,
}

#[derive(Deserialize)]
struct PostAction {
    action: Action,
}

#[derive(Serialize)]
struct LegalAction {
    action: Action,
    text: String,
}

/// Everything the page draws. Built fresh for every response.
#[derive(Serialize)]
struct View<'a> {
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
    p.kind().map_or_else(|| "GameOver".to_string(), |k| format!("{k:?}"))
}

fn view<'a>(id: &'a str, s: &'a Session) -> View<'a> {
    view_with(id, s, None)
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
        (Phase::Trading { offer, .. }, Some(_)) if human_to_act => (offer.is_none(), offer.is_some()),
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
        stars_held: (0..g.num_players()).map(|i| g.stars_held(PlayerId(i))).collect(),
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

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn json_view(id: &str, s: &Session) -> Response {
    Json(view(id, s)).into_response()
}

fn json_view_trail(id: &str, s: &Session, trail: Vec<Frame>) -> Response {
    let mut v = view(id, s);
    v.trail = trail;
    Json(v).into_response()
}

async fn get_config(State(server): State<Shared>) -> Response {
    Json(server.config.as_ref()).into_response()
}

async fn create_game(State(server): State<Shared>, Json(req): Json<NewGame>) -> Response {
    let seed = req.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
    });
    let game = match Game::new(server.config.clone(), req.seats.len(), seed) {
        Ok(g) => g,
        Err(e) => return error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let bots = make_bots(&req.seats, seed);
    let mut session = Session {
        game,
        seats: req.seats,
        bots,
        log: Vec::new(),
    };
    session.log.push(LogEntry {
        n: 0,
        round: 1,
        kind: "round",
        text: format!("New game, seed {seed}"),
    });
    let trail = tokio::task::block_in_place(|| session.run_bots());

    let mut games = server.games.lock().unwrap();
    let id = format!("{:x}", seed ^ (games.len() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let response = json_view_trail(&id, &session, trail);
    games.insert(id, session);
    response
}

async fn get_game(State(server): State<Shared>, Path(id): Path<String>) -> Response {
    let games = server.games.lock().unwrap();
    match games.get(&id) {
        Some(s) => json_view(&id, s),
        None => error(StatusCode::NOT_FOUND, "no such game"),
    }
}

async fn post_action(State(server): State<Shared>, Path(id): Path<String>, Json(req): Json<PostAction>) -> Response {
    let mut games = server.games.lock().unwrap();
    let Some(s) = games.get_mut(&id) else {
        return error(StatusCode::NOT_FOUND, "no such game");
    };
    let Some(p) = s.game.current_player() else {
        return error(StatusCode::CONFLICT, "the game is over");
    };
    if s.seats[p.0] != SeatKind::Human {
        return error(StatusCode::CONFLICT, "it's a bot's turn");
    }
    match s.step(req.action) {
        Ok(first) => {
            let mut trail = vec![first];
            trail.extend(tokio::task::block_in_place(|| s.run_bots()));
            json_view_trail(&id, s, trail)
        }
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// Everything needed to rebuild a game: replaying `history` from `seed`.
#[derive(Serialize, Deserialize)]
struct SavedGame {
    seats: Vec<SeatKind>,
    seed: u64,
    history: Vec<Action>,
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

async fn save_game(State(server): State<Shared>, Path(id): Path<String>) -> Response {
    let games = server.games.lock().unwrap();
    match games.get(&id) {
        Some(s) => Json(SavedGame {
            seats: s.seats.clone(),
            seed: s.game.seed(),
            history: s.game.history().to_vec(),
        })
        .into_response(),
        None => error(StatusCode::NOT_FOUND, "no such game"),
    }
}

fn insert_session(server: &Server, id: Option<String>, mut session: Session) -> Response {
    tokio::task::block_in_place(|| session.run_bots());
    let mut games = server.games.lock().unwrap();
    let id = id.unwrap_or_else(|| {
        format!("{:x}", session.game.seed() ^ (games.len() as u64 + 1).wrapping_mul(0xbf58_476d_1ce4_e5b9))
    });
    let response = json_view(&id, &session);
    games.insert(id, session);
    response
}

async fn load_game(State(server): State<Shared>, Json(req): Json<LoadGame>) -> Response {
    let SavedGame { seats, seed, history } = req.saved;
    match Session::replay(server.config.clone(), seats, seed, &history) {
        Ok(session) => insert_session(&server, req.id, session),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

/// Rebuilds a game's history from its seed and log text, for games on a
/// server that had no save endpoint. Returns a `SavedGame`.
async fn recover_game(State(server): State<Shared>, Json(req): Json<RecoverGame>) -> Response {
    let game = match Game::new(server.config.clone(), req.seats.len(), req.seed) {
        Ok(g) => g,
        Err(e) => return error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let human: Vec<bool> = req.seats.iter().map(|k| *k == SeatKind::Human).collect();
    match tokio::task::block_in_place(|| recover::recover(game, &req.log, &human)) {
        Ok(r) => Json(SavedGame {
            seats: req.seats,
            seed: req.seed,
            history: r.history,
        })
        .into_response(),
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e),
    }
}

#[derive(Deserialize)]
struct StepQuery {
    step: usize,
}

/// The game as it stood after its first `step` actions, read-only, for
/// stepping through a game. Bots aren't consulted: the history is replayed.
async fn get_step(State(server): State<Shared>, Path(id): Path<String>, Query(q): Query<StepQuery>) -> Response {
    let (seats, seed, history) = {
        let games = server.games.lock().unwrap();
        let Some(s) = games.get(&id) else {
            return error(StatusCode::NOT_FOUND, "no such game");
        };
        (s.seats.clone(), s.game.seed(), s.game.history().to_vec())
    };
    let step = q.step.min(history.len());
    let mut g = match Game::new(server.config.clone(), seats.len(), seed) {
        Ok(g) => g,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let mut marks = vec![Mark {
        step: 0,
        round: g.round(),
        phase: phase_label(g.phase()),
    }];
    let mut at_step = None;
    let mut last = None;
    let mut next = None;
    let mut log = vec![LogEntry {
        n: 0,
        round: 1,
        kind: "round",
        text: format!("New game, seed {seed}"),
    }];
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
        let events = match g.apply(a.clone()) {
            Ok(ev) => ev,
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        if i < step {
            for e in &events {
                let kind = match e {
                    GameEvent::RoundStarted { .. } => "round",
                    GameEvent::PhaseStarted { .. } => "phase",
                    _ => "event",
                };
                log.push(LogEntry {
                    n: log.len(),
                    round: g.round(),
                    kind,
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
    let (game, log) = at_step.unwrap_or((g, log));
    let session = Session {
        game,
        bots: seats.iter().map(|_| None).collect(),
        seats,
        log,
    };
    let info = ReplayInfo {
        step,
        total: history.len(),
        marks,
        last,
        next,
    };
    Json(view_with(&id, &session, Some(info))).into_response()
}

/// Takes back the last human action and everything after it.
async fn undo(State(server): State<Shared>, Path(id): Path<String>) -> Response {
    let mut games = server.games.lock().unwrap();
    let Some(s) = games.get_mut(&id) else {
        return error(StatusCode::NOT_FOUND, "no such game");
    };
    // Who made each action: replay and ask before each one.
    let history = s.game.history().to_vec();
    let mut g = match Game::new(server.config.clone(), s.seats.len(), s.game.seed()) {
        Ok(g) => g,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let mut last_human = None;
    for (i, a) in history.iter().enumerate() {
        if g.current_player().is_some_and(|p| s.seats[p.0] == SeatKind::Human) {
            last_human = Some(i);
        }
        g.apply(a.clone()).expect("history replays");
    }
    let Some(cut) = last_human else {
        return error(StatusCode::CONFLICT, "nothing to undo");
    };
    match Session::replay(server.config.clone(), s.seats.clone(), s.game.seed(), &history[..cut]) {
        Ok(session) => {
            *s = session;
            json_view(&id, s)
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    // no-cache: after a rebuild the browser must not keep the old page.
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let config = match &cli.data {
        Some(dir) => GameConfig::from_dir(dir).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        }),
        None => GameConfig::embedded(),
    };
    let server = Arc::new(Server {
        config: Arc::new(config),
        games: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/", get(|| async { asset("text/html; charset=utf-8", INDEX_HTML) }))
        .route("/app.js", get(|| async { asset("text/javascript; charset=utf-8", APP_JS) }))
        .route("/icons.js", get(|| async { asset("text/javascript; charset=utf-8", ICONS_JS) }))
        .route("/style.css", get(|| async { asset("text/css; charset=utf-8", STYLE_CSS) }))
        .route("/m", get(|| async { asset("text/html; charset=utf-8", M_INDEX_HTML) }))
        .route("/m/", get(|| async { asset("text/html; charset=utf-8", M_INDEX_HTML) }))
        .route("/m/app.js", get(|| async { asset("text/javascript; charset=utf-8", M_APP_JS) }))
        .route("/m/style.css", get(|| async { asset("text/css; charset=utf-8", M_STYLE_CSS) }))
        .route("/api/config", get(get_config))
        .route("/api/games", post(create_game))
        .route("/api/games/{id}", get(get_game))
        .route("/api/games/{id}/actions", post(post_action))
        .route("/api/games/{id}/save", get(save_game))
        .route("/api/games/{id}/undo", post(undo))
        .route("/api/games/{id}/step", get(get_step))
        .route("/api/games/load", post(load_game))
        .route("/api/games/recover", post(recover_game))
        .with_state(server);

    let addr = format!("{}:{}", cli.host, cli.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        eprintln!("can't listen on {addr}: {e}");
        std::process::exit(1);
    });
    println!("Colosseum is running at http://{addr}");
    axum::serve(listener, app).await.expect("server error");
}
