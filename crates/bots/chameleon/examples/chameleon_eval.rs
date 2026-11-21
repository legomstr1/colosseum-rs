//! Evaluates chameleon against planner, auctioneer and trader.
//!
//! `cargo run --release -p colosseum-bot-chameleon --example chameleon_eval [games_per_cell] [timing_games] [timing_reps]`
//!
//! - Mixed: every seat drawn (with repeats) from all four bots, at least one
//!   chameleon, at 3, 4 and 5 players.
//! - Head-to-head: seats drawn from {chameleon, X}, both present.
//! - Timing: 5-player games on one thread, all-chameleon and mixed.
//!
//! Win rate is reported against fair share: wins / sum of 1/players over
//! the bot's seats. `CHAM_TUNE="deny=0.4,..."` overrides chameleon's knobs.

use colosseum_bot_auctioneer::AuctioneerAgent;
use colosseum_bot_chameleon::{ChameleonAgent, Params};
use colosseum_bot_planner::PlannerAgent;
use colosseum_bot_trader::TraderAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const BOTS: [&str; 4] = ["chameleon", "planner", "auctioneer", "trader"];

fn params() -> Params {
    Params::default().with_overrides(&std::env::var("CHAM_TUNE").unwrap_or_default())
}

fn make(name: &str, seed: u64) -> Box<dyn Agent> {
    match name {
        "chameleon" => Box::new(ChameleonAgent::with_params(params())),
        "planner" => Box::new(PlannerAgent::new(seed)),
        "auctioneer" => Box::new(AuctioneerAgent::new(seed)),
        _ => Box::new(TraderAgent::new(seed)),
    }
}

struct Played {
    winner: usize,
    actions: u64,
    wall: Duration,
    cham_think: Duration,
    cham_decisions: u64,
}

fn play(config: &Arc<GameConfig>, seats: &[&str], seed: u64) -> Played {
    let n = seats.len();
    let mut game = Game::new(config.clone(), n, seed).unwrap();
    let mut agents: Vec<Box<dyn Agent>> = seats
        .iter()
        .enumerate()
        .map(|(i, s)| make(s, seed.wrapping_mul(31).wrapping_add(i as u64)))
        .collect();
    let mut cham_think = Duration::ZERO;
    let mut cham_decisions = 0;
    let start = Instant::now();
    let mut steps = 0u32;
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let t = Instant::now();
        let a = agents[p.0].choose_action(&game, &legal);
        if seats[p.0] == "chameleon" {
            cham_think += t.elapsed();
            cham_decisions += 1;
        }
        if let Err(e) = game.apply(a) {
            panic!("{} chose an illegal action: {e} (seed {seed})", seats[p.0]);
        }
        steps += 1;
        assert!(steps < 200_000, "game did not terminate (seed {seed})");
    }
    let wall = start.elapsed();
    Played {
        winner: game.ranking()[0].0,
        actions: steps as u64,
        wall,
        cham_think,
        cham_decisions,
    }
}

/// Draws seats from `pool` with repeats until `ok` holds.
fn draw<'a>(rng: &mut Rng, n: usize, pool: &[&'a str], ok: impl Fn(&[&str]) -> bool) -> Vec<&'a str> {
    loop {
        let s: Vec<&str> = (0..n).map(|_| pool[rng.below(pool.len())]).collect();
        if ok(&s) {
            return s;
        }
    }
}

/// Per-bot `(wins, fair)` over many games, in parallel.
fn run(config: &Arc<GameConfig>, n: usize, pool: &[&'static str], games: u64, seed0: u64) -> Vec<(f64, f64)> {
    let threads = std::thread::available_parallelism().map_or(4, |x| x.get()) as u64;
    let next = Arc::new(AtomicU64::new(0));
    let pool: Vec<&'static str> = pool.to_vec();
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let config = config.clone();
            let next = next.clone();
            let pool = pool.clone();
            std::thread::spawn(move || {
                let mut tally = vec![(0.0f64, 0.0f64); BOTS.len()];
                loop {
                    let g = next.fetch_add(1, Ordering::Relaxed);
                    if g >= games {
                        break;
                    }
                    let seed = seed0.wrapping_add(g.wrapping_mul(0x9E37_79B9));
                    let mut rng = Rng::new(seed ^ 0xC0FFEE);
                    let seats = draw(&mut rng, n, &pool, |s| {
                        s.contains(&"chameleon") && (pool.len() > 2 || s.iter().any(|b| *b != "chameleon"))
                    });
                    let r = play(&config, &seats, seed);
                    for s in &seats {
                        let i = BOTS.iter().position(|b| b == s).unwrap();
                        tally[i].1 += 1.0 / n as f64;
                    }
                    let wi = BOTS.iter().position(|b| *b == seats[r.winner]).unwrap();
                    tally[wi].0 += 1.0;
                }
                tally
            })
        })
        .collect();
    let mut total = vec![(0.0, 0.0); BOTS.len()];
    for h in handles {
        for (i, (w, f)) in h.join().unwrap().into_iter().enumerate() {
            total[i].0 += w;
            total[i].1 += f;
        }
    }
    total
}

fn ratio(x: (f64, f64)) -> f64 {
    if x.1 > 0.0 { x.0 / x.1 } else { 0.0 }
}

#[derive(Clone, Copy)]
enum Lineup {
    AllChameleon,
    Mixed,
    /// Only the three existing bots: a calibration for machine load.
    Reference,
}

/// Times 5-player games on this thread. Each game is deterministic, so it
/// is replayed `reps` times and the fastest run is kept, which filters out
/// interference from other processes.
fn timing(config: &Arc<GameConfig>, games: u64, kind: Lineup, reps: u32) -> (f64, f64, f64, f64) {
    let mut walls = Vec::new();
    let mut think = 0.0f64;
    let mut decisions = 0u64;
    let mut actions = 0u64;
    for g in 0..games {
        let seed = 777_000 + g;
        let mut rng = Rng::new(seed);
        let seats = match kind {
            Lineup::Mixed => draw(&mut rng, 5, &BOTS, |s| s.contains(&"chameleon") && s.iter().any(|b| *b != "chameleon")),
            Lineup::AllChameleon => vec!["chameleon"; 5],
            Lineup::Reference => draw(&mut rng, 5, &BOTS[1..], |_| true),
        };
        let mut best: Option<Played> = None;
        for _ in 0..reps.max(1) {
            let r = play(config, &seats, seed);
            if best.as_ref().is_none_or(|b| r.wall < b.wall) {
                best = Some(r);
            }
        }
        let r = best.expect("at least one run");
        walls.push(r.wall.as_secs_f64() * 1000.0);
        think += r.cham_think.as_secs_f64() * 1000.0;
        decisions += r.cham_decisions;
        actions += r.actions;
    }
    walls.sort_by(f64::total_cmp);
    let mean = walls.iter().sum::<f64>() / walls.len().max(1) as f64;
    let p95 = walls[((walls.len() as f64 * 0.95) as usize).min(walls.len() - 1)];
    (mean, p95, think / decisions.max(1) as f64, actions as f64 / games.max(1) as f64)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let tgames: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let reps: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    let only = std::env::var("CHAM_ONLY").unwrap_or_default();
    let config = Arc::new(GameConfig::embedded());

    if only.is_empty() || only.contains("mixed") {
        println!("== Mixed (seats from all four, with repeats, >=1 chameleon), {games} games per size");
        println!("{:>3} {:>10} {:>10} {:>11} {:>10}", "n", "chameleon", "planner", "auctioneer", "trader");
        for n in 3..=5 {
            let t = run(&config, n, &BOTS, games, 1_000_000 * n as u64);
            println!(
                "{n:>3} {:>10.3} {:>10.3} {:>11.3} {:>10.3}",
                ratio(t[0]),
                ratio(t[1]),
                ratio(t[2]),
                ratio(t[3])
            );
        }
    }
    if only.is_empty() || only.contains("h2h") {
        println!("== Head-to-head (seats from {{chameleon, X}}, both present): chameleon wins / fair share");
        println!("{:>3} {:>10} {:>11} {:>10}", "n", "planner", "auctioneer", "trader");
        for n in 3..=5 {
            let mut row = Vec::new();
            for x in ["planner", "auctioneer", "trader"] {
                let t = run(&config, n, &["chameleon", x], games, 7_000_000 * n as u64 + x.len() as u64);
                row.push(ratio(t[0]));
            }
            println!("{n:>3} {:>10.3} {:>11.3} {:>10.3}", row[0], row[1], row[2]);
        }
    }
    if (only.is_empty() || only.contains("time")) && tgames > 0 {
        println!("== Timing: 5-player games on one thread, {tgames} games each, best of {reps} runs per game");
        let (m, p, _, a) = timing(&config, tgames, Lineup::Reference, reps);
        println!("reference (planner/auctioneer/trader only): mean {m:.2} ms, p95 {p:.2} ms, {a:.0} actions/game");
        let (m, p, d, a) = timing(&config, tgames, Lineup::AllChameleon, reps);
        println!("all-chameleon: mean {m:.2} ms, p95 {p:.2} ms, {a:.0} actions/game, chameleon {d:.4} ms/decision");
        let (m, p, d, a) = timing(&config, tgames, Lineup::Mixed, reps);
        println!("mixed:         mean {m:.2} ms, p95 {p:.2} ms, {a:.0} actions/game, chameleon {d:.4} ms/decision");
    }
}
