//! Plays maestro against planner, auctioneer and trader.
//!
//! `cargo run --release -p colosseum-bot-maestro --example maestro_eval -- [mode] [games] [key=value ...]`
//!
//! Modes:
//! - `pool`: every seat drawn (with repeats) from {maestro, planner,
//!   auctioneer, trader}, at least one maestro; 3, 4 and 5 players.
//! - `h2h`: for each opponent X, seats drawn from {maestro, X} with both
//!   present; 3, 4 and 5 players.
//! - `time`: single-threaded 5-player wall time per game, all-maestro and
//!   mixed with the other three.
//! - `all` (default): all of the above.
//!
//! Win rates are reported against fair share: wins divided by the sum of
//! 1/players over the bot's seats. A win is first place in `Game::ranking`.
//! `key=value` arguments override maestro's tuning parameters.

use colosseum_bot_auctioneer::AuctioneerAgent;
use colosseum_bot_maestro::{MaestroAgent, Params};
use colosseum_bot_planner::PlannerAgent;
use colosseum_bot_trader::TraderAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const BOTS: [&str; 4] = ["maestro", "planner", "auctioneer", "trader"];

fn make(bot: usize, seed: u64, params: &Params) -> Box<dyn Agent> {
    match bot {
        0 => Box::new(MaestroAgent::with_params(seed, params.clone())),
        1 => Box::new(PlannerAgent::new(seed)),
        2 => Box::new(AuctioneerAgent::new(seed)),
        _ => Box::new(TraderAgent::new(seed)),
    }
}

/// Plays one game; returns the winner's seat, the seat bots and the scores.
fn play(config: &Arc<GameConfig>, seats: &[usize], seed: u64, params: &Params) -> (usize, Vec<u32>) {
    let n = seats.len();
    let mut game = Game::new(config.clone(), n, seed).unwrap();
    let mut agents: Vec<Box<dyn Agent>> = seats
        .iter()
        .enumerate()
        .map(|(i, &b)| make(b, seed.wrapping_mul(31).wrapping_add(i as u64), params))
        .collect();
    let mut steps = 0u32;
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let a = agents[p.0].choose_action(&game, &legal);
        if let Err(e) = game.apply(a) {
            panic!("seat {} ({}) illegal action: {e} [seed {seed}, seats {seats:?}]", p.0, BOTS[seats[p.0]]);
        }
        steps += 1;
        assert!(steps < 200_000, "game did not terminate");
    }
    let winner = game.ranking()[0].0;
    (winner, game.players().iter().map(|p| p.best_score).collect())
}

#[derive(Default, Clone)]
struct Tally {
    games: u64,
    wins: [f64; 4],
    fair: [f64; 4],
    score: [f64; 4],
    seats: [f64; 4],
    win_score: f64,
}

impl Tally {
    fn merge(&mut self, o: &Tally) {
        self.games += o.games;
        self.win_score += o.win_score;
        for b in 0..4 {
            self.wins[b] += o.wins[b];
            self.fair[b] += o.fair[b];
            self.score[b] += o.score[b];
            self.seats[b] += o.seats[b];
        }
    }
    fn ratio(&self, b: usize) -> f64 {
        self.wins[b] / self.fair[b].max(1e-9)
    }
}

/// Runs `games` games of `n` players with seats drawn from `pool`, at least
/// one maestro and (for head-to-head) at least one of every pool bot.
fn run(config: &Arc<GameConfig>, n: usize, pool: &[usize], games: u64, seed0: u64, params: &Params) -> Tally {
    let threads = std::thread::available_parallelism().map_or(4, |x| x.get()) as u64;
    let next = AtomicU64::new(0);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut t = Tally::default();
                    loop {
                        let g = next.fetch_add(1, Ordering::Relaxed);
                        if g >= games {
                            break;
                        }
                        let seed = seed0.wrapping_mul(1_000_003).wrapping_add(g * 7919 + n as u64);
                        let mut rng = Rng::new(seed ^ 0x5EA7_5EA7);
                        let seats: Vec<usize> = loop {
                            let s: Vec<usize> = (0..n).map(|_| pool[rng.below(pool.len())]).collect();
                            let all = pool.len() > 2 || pool.iter().all(|b| s.contains(b));
                            if s.contains(&0) && all {
                                break s;
                            }
                        };
                        let (winner, scores) = play(config, &seats, seed, params);
                        t.games += 1;
                        t.wins[seats[winner]] += 1.0;
                        t.win_score += scores[winner] as f64;
                        for (i, &b) in seats.iter().enumerate() {
                            t.fair[b] += 1.0 / n as f64;
                            t.score[b] += scores[i] as f64;
                            t.seats[b] += 1.0;
                        }
                    }
                    t
                })
            })
            .collect();
        let mut total = Tally::default();
        for h in handles {
            total.merge(&h.join().unwrap());
        }
        total
    })
}

fn report(label: &str, t: &Tally, pool: &[usize]) {
    let mut line = format!("{label:<22} games {:>6}  win score {:5.1} |", t.games, t.win_score / t.games.max(1) as f64);
    for &b in pool {
        line += &format!(
            " {} {:.3}x (avg {:4.1})",
            BOTS[b],
            t.ratio(b),
            t.score[b] / t.seats[b].max(1.0)
        );
    }
    println!("{line}");
}

fn time_games(config: &Arc<GameConfig>, games: u64, mixed: bool, params: &Params, without: bool) -> (f64, f64, f64, [f64; 4]) {
    let mut walls = Vec::new();
    let mut actions = 0u64;
    let mut think = [0f64; 4];
    let mut offers = [0f64; 4];
    for g in 0..games {
        let seed = 900_000 + g;
        let mut rng = Rng::new(seed ^ 0x7177);
        let seats: Vec<usize> = if without {
            (0..5).map(|_| 1 + rng.below(3)).collect()
        } else if mixed {
            loop {
                let s: Vec<usize> = (0..5).map(|_| rng.below(4)).collect();
                if s.contains(&0) {
                    break s;
                }
            }
        } else {
            vec![0; 5]
        };
        let start = Instant::now();
        let mut game = Game::new(config.clone(), 5, seed).unwrap();
        let mut agents: Vec<Box<dyn Agent>> =
            seats.iter().enumerate().map(|(i, &b)| make(b, seed + i as u64, params)).collect();
        while let Some(p) = game.current_player() {
            let legal = game.legal_actions();
            let t = Instant::now();
            let a = agents[p.0].choose_action(&game, &legal);
            think[seats[p.0]] += t.elapsed().as_secs_f64() * 1000.0 / games as f64;
            if matches!(a, colosseum_engine::Action::ProposeTrade { .. } | colosseum_engine::Action::CounterTrade { .. }) {
                offers[seats[p.0]] += 1.0 / games as f64;
            }
            game.apply(a).expect("legal");
            actions += 1;
        }
        walls.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    walls.sort_by(|a, b| a.total_cmp(b));
    let mean = walls.iter().sum::<f64>() / walls.len() as f64;
    let p95 = walls[((walls.len() as f64 * 0.95) as usize).min(walls.len() - 1)];
    if mixed {
        let parts: Vec<String> = (0..4).map(|b| format!("{} {:.1}", BOTS[b], offers[b])).collect();
        println!("  mixed offers/game by bot: {}", parts.join(", "));
    }
    (mean, p95, actions as f64 / games as f64, think)
}

/// Single-threaded maestro decision time in all-maestro 5-player games.
fn decision_time(config: &Arc<GameConfig>, games: u64, params: &Params) -> f64 {
    let mut nanos = 0u128;
    let mut calls = 0u64;
    let mut ph = [(0u128, 0u64); 6];
    for g in 0..games {
        let seed = 800_000 + g;
        let mut game = Game::new(config.clone(), 5, seed).unwrap();
        let mut agents: Vec<MaestroAgent> = (0..5).map(|i| MaestroAgent::with_params(seed + i, params.clone())).collect();
        while let Some(p) = game.current_player() {
            let legal = game.legal_actions();
            let t = Instant::now();
            let a = agents[p.0].choose_action(&game, &legal);
            let dt = t.elapsed().as_nanos();
            nanos += dt;
            calls += 1;
            let k = game.phase().kind().map_or(5, |k| k as usize);
            ph[k].0 += dt;
            ph[k].1 += 1;
            game.apply(a).expect("legal");
        }
    }
    let names = ["invest", "acquire", "trade", "produce", "close", "-"];
    let parts: Vec<String> = (0..5)
        .map(|k| format!("{} {:.1}/game x {:.4} ms", names[k], ph[k].1 as f64 / games as f64, ph[k].0 as f64 / ph[k].1.max(1) as f64 / 1e6))
        .collect();
    println!("  by phase (all-maestro): {}", parts.join(", "));
    nanos as f64 / calls.max(1) as f64 / 1e6
}

fn main() {
    let mut params = Params::default();
    let mut positional = Vec::new();
    for a in std::env::args().skip(1) {
        if let Some((k, v)) = a.split_once('=') {
            if !params.set(k, v.parse().expect("numeric value")) {
                panic!("unknown parameter {k}");
            }
        } else {
            positional.push(a);
        }
    }
    let mode = positional.first().cloned().unwrap_or_else(|| "all".into());
    let games: u64 = positional.get(1).and_then(|s| s.parse().ok()).unwrap_or(3000);
    let config = Arc::new(GameConfig::embedded());
    let start = Instant::now();

    if mode == "all" || mode == "pool" {
        println!("== pool: seats drawn from all four bots (with repeats), >= 1 maestro ==");
        let mut all = Tally::default();
        for n in [3usize, 4, 5] {
            let t = run(&config, n, &[0, 1, 2, 3], games, 11, &params);
            report(&format!("{n}p pool"), &t, &[0, 1, 2, 3]);
            all.merge(&t);
        }
        report("all sizes pool", &all, &[0, 1, 2, 3]);
    }
    // `h2h:<bot>` restricts the head-to-head to one opponent.
    let only = mode.strip_prefix("h2h:").and_then(|b| BOTS.iter().position(|x| *x == b));
    if mode == "all" || mode == "h2h" || only.is_some() {
        println!("== head-to-head: seats drawn from {{maestro, X}}, both present ==");
        for x in (1..4).filter(|x| only.is_none_or(|o| o == *x)) {
            let mut all = Tally::default();
            for n in [3usize, 4, 5] {
                let t = run(&config, n, &[0, x], games, 23 + x as u64, &params);
                report(&format!("{n}p vs {}", BOTS[x]), &t, &[0, x]);
                all.merge(&t);
            }
            report(&format!("all vs {}", BOTS[x]), &all, &[0, x]);
        }
    }
    if mode == "all" || mode == "time" {
        let tg = (games / 10).clamp(100, 1000);
        println!("== timing: 5 players, one thread, {tg} games each ==");
        let (m, p95, acts, _) = time_games(&config, tg, false, &params, false);
        println!("all-maestro: mean {m:.2} ms  p95 {p95:.2} ms  ({acts:.0} actions/game)");
        let (m, p95, acts, _) = time_games(&config, tg, true, &params, true);
        println!("reference (no maestro): mean {m:.2} ms  p95 {p95:.2} ms  ({acts:.0} actions/game)");
        let (m, p95, acts, think) = time_games(&config, tg, true, &params, false);
        println!("mixed:       mean {m:.2} ms  p95 {p95:.2} ms  ({acts:.0} actions/game)");
        let parts: Vec<String> = (0..4).map(|b| format!("{} {:.2}", BOTS[b], think[b])).collect();
        println!("  mixed think ms/game by bot: {}", parts.join(", "));
        println!("maestro decision: {:.4} ms", decision_time(&config, tg.min(300), &params));
    }
    eprintln!("({:.1}s)", start.elapsed().as_secs_f64());
}
