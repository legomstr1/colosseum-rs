//! `colosseum-web`: serves a browser UI and a small JSON API over the engine.
//!
//! The server owns every game. A browser creates a game, reads its view and
//! posts actions; bots move on the server until a human seat is to act. Seats
//! are part of each session so that remote players can join later.

use axum::extract::{Path, State};
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

const INDEX_HTML: &str = include_str!("../static/index.html");
const APP_JS: &str = include_str!("../static/app.js");
const ICONS_JS: &str = include_str!("../static/icons.js");
const STYLE_CSS: &str = include_str!("../static/style.css");

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

impl Session {
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

    /// Lets bots act until a human seat must decide or the game ends.
    fn run_bots(&mut self) {
        while let Some(p) = self.game.current_player() {
            let Some(bot) = self.bots[p.0].as_mut() else {
                return;
            };
            let legal = self.game.legal_actions();
            let action = bot.choose_action(&self.game, &legal);
            let events = self.game.apply(action).expect("bots only pick legal actions");
            self.record(&events);
        }
    }
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
}

fn view<'a>(id: &'a str, s: &'a Session) -> View<'a> {
    let g = &s.game;
    let current = g.current_player();
    let human_to_act = current.is_some_and(|p| s.seats[p.0] == SeatKind::Human);
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
    }
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn json_view(id: &str, s: &Session) -> Response {
    Json(view(id, s)).into_response()
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
    let bots = req
        .seats
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
        .collect();
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
    tokio::task::block_in_place(|| session.run_bots());

    let mut games = server.games.lock().unwrap();
    let id = format!("{:x}", seed ^ (games.len() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let response = json_view(&id, &session);
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
    match s.game.apply(req.action) {
        Ok(events) => {
            s.record(&events);
            tokio::task::block_in_place(|| s.run_bots());
            json_view(&id, s)
        }
        Err(e) => error(StatusCode::UNPROCESSABLE_ENTITY, e.reason),
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
        .route("/api/config", get(get_config))
        .route("/api/games", post(create_game))
        .route("/api/games/{id}", get(get_game))
        .route("/api/games/{id}/actions", post(post_action))
        .with_state(server);

    let addr = format!("{}:{}", cli.host, cli.port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        eprintln!("can't listen on {addr}: {e}");
        std::process::exit(1);
    });
    println!("Colosseum is running at http://{addr}");
    axum::serve(listener, app).await.expect("server error");
}
