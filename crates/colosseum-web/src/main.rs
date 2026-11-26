//! `colosseum-web`: serves the browser UI and its JSON API.
//!
//! The API lives in `colosseum-session` so the WebAssembly build can serve
//! the same requests inside the browser. This binary adds every bot, real
//! threads for the search bots, and HTTP.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use clap::Parser;
use colosseum_engine::GameConfig;
use colosseum_engine::agent::{GreedyAgent, RandomAgent};
use colosseum_session::{Api, Bot, SeatKind};
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

/// Every seat kind the setup screen offers, in menu order.
const KINDS: &[SeatKind] = &[
    SeatKind::Human,
    SeatKind::LiveInput,
    SeatKind::Tuned2,
    SeatKind::Tuned1,
    SeatKind::IntEvent,
    SeatKind::Search,
    SeatKind::House,
    SeatKind::Oracle,
    SeatKind::Maestro,
    SeatKind::Chameleon,
    SeatKind::Trader,
    SeatKind::Planner,
    SeatKind::Auctioneer,
    SeatKind::Showman,
    SeatKind::Rollout,
    SeatKind::Greedy,
    SeatKind::Random,
];

fn make_bot(kind: SeatKind, bot_seed: u64) -> Option<Bot> {
    match kind {
        SeatKind::Human => None,
        SeatKind::Search => {
            // Results don't depend on the thread count, only the speed.
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let cfg = colosseum_bot_search::Config {
                threads,
                ..Default::default()
            };
            Some(Box::new(colosseum_bot_search::SearchAgent::with_config(
                bot_seed, cfg,
            )))
        }
        SeatKind::LiveInput => {
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
            let cfg = colosseum_bot_live_input::SearchConfig {
                threads,
                ..Default::default()
            };
            Some(Box::new(
                colosseum_bot_live_input::LiveInputAgent::with_config(
                    bot_seed,
                    Default::default(),
                    cfg,
                ),
            ))
        }
        SeatKind::Tuned1 => Some(Box::new(colosseum_bot_tuned_1::Tuned1Agent::new(bot_seed))),
        SeatKind::Tuned2 => Some(Box::new(colosseum_bot_tuned_2::Tuned2Agent::new(bot_seed))),
        SeatKind::IntEvent => Some(Box::new(colosseum_bot_int_event::IntEventAgent::new(
            bot_seed,
        ))),
        SeatKind::House => Some(Box::new(colosseum_bot_house::HouseAgent::new(bot_seed))),
        SeatKind::Greedy => Some(Box::new(GreedyAgent::new(bot_seed))),
        SeatKind::Random => Some(Box::new(RandomAgent::new(bot_seed))),
        SeatKind::Oracle => Some(Box::new(colosseum_bot_oracle::OracleAgent::new(bot_seed))),
        SeatKind::Maestro => Some(Box::new(colosseum_bot_maestro::MaestroAgent::new(bot_seed))),
        SeatKind::Chameleon => Some(Box::new(colosseum_bot_chameleon::ChameleonAgent::new(
            bot_seed,
        ))),
        SeatKind::Trader => Some(Box::new(colosseum_bot_trader::TraderAgent::new(bot_seed))),
        SeatKind::Planner => Some(Box::new(colosseum_bot_planner::PlannerAgent::new(bot_seed))),
        SeatKind::Auctioneer => Some(Box::new(colosseum_bot_auctioneer::AuctioneerAgent::new(
            bot_seed,
        ))),
        SeatKind::Showman => Some(Box::new(colosseum_bot_showman::ShowmanAgent::new(bot_seed))),
        SeatKind::Rollout => Some(Box::new(colosseum_bot_rollout::RolloutAgent::new(bot_seed))),
    }
}

/// At most 53 bits: the page reads seeds as JavaScript numbers, so a
/// larger one would come back changed from a saved game.
fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
        & ((1 << 53) - 1)
}

type Shared = Arc<Mutex<Api>>;

async fn api(
    State(api): State<Shared>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Response {
    let path = uri
        .path_and_query()
        .map_or(uri.path(), |p| p.as_str())
        .to_string();
    let body = String::from_utf8_lossy(&body);
    // Bots think inside the request.
    let (status, json) =
        tokio::task::block_in_place(|| api.lock().unwrap().route(method.as_str(), &path, &body));
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [(header::CONTENT_TYPE, "application/json")],
        json,
    )
        .into_response()
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    // no-cache: after a rebuild the browser must not keep the old page.
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
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
    let api_state: Shared = Arc::new(Mutex::new(Api::new(
        config,
        KINDS.to_vec(),
        make_bot,
        fresh_seed,
    )));

    let app = Router::new()
        .route(
            "/",
            get(|| async { asset("text/html; charset=utf-8", INDEX_HTML) }),
        )
        .route(
            "/app.js",
            get(|| async { asset("text/javascript; charset=utf-8", APP_JS) }),
        )
        .route(
            "/icons.js",
            get(|| async { asset("text/javascript; charset=utf-8", ICONS_JS) }),
        )
        .route(
            "/style.css",
            get(|| async { asset("text/css; charset=utf-8", STYLE_CSS) }),
        )
        .route("/api/{*rest}", get(api).post(api))
        .with_state(api_state);

    let addr = format!("{}:{}", cli.host, cli.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("can't listen on {addr}: {e}");
            std::process::exit(1);
        });
    println!("Colosseum is running at http://{addr}");
    axum::serve(listener, app).await.expect("server error");
}
