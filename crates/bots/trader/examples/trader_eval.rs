//! Plays the trader against the baseline bots and reports win rates, trades
//! per game and time per decision.
//!
//! `cargo run --release -p colosseum-bot-trader --example trader_eval [games_per_cell]`

use colosseum_bot_trader::TraderAgent;
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::{Action, Game, GameConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static PH_NANOS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static PH_CALLS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

struct Timed {
    inner: TraderAgent,
    nanos: Arc<AtomicU64>,
    calls: Arc<AtomicU64>,
}

impl Agent for Timed {
    fn name(&self) -> String {
        self.inner.name()
    }
    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let t = Instant::now();
        let a = self.inner.choose_action(game, legal);
        self.nanos.fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.calls.fetch_add(1, Ordering::Relaxed);
        let k = game.phase().kind().map_or(5, |k| k as usize);
        PH_NANOS[k].fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
        PH_CALLS[k].fetch_add(1, Ordering::Relaxed);
        a
    }
}

#[derive(Default, Clone, Copy)]
struct Tally {
    games: u64,
    trader_seats: u64,
    wins: f64,
    trades: u64,
    nanos: u64,
    calls: u64,
}

/// `traders` seats (rotated) play TraderAgent; the rest play `other`.
fn run(config: &Arc<GameConfig>, n: usize, traders: usize, other: &str, games: u64, seed0: u64, passive: bool) -> Tally {
    let threads = std::thread::available_parallelism().map_or(4, |x| x.get());
    let mut handles = Vec::new();
    for t in 0..threads as u64 {
        let config = config.clone();
        let other = other.to_string();
        handles.push(std::thread::spawn(move || {
            let mut tally = Tally::default();
            let mut g = t;
            while g < games {
                let seed = seed0 + g * 7919;
                let mut game = Game::new(config.clone(), n, seed).unwrap();
                let nanos = Arc::new(AtomicU64::new(0));
                let calls = Arc::new(AtomicU64::new(0));
                let rot = (g as usize) % n;
                let is_trader = |s: usize| (s + n - rot) % n < traders;
                let mut agents: Vec<Box<dyn Agent>> = (0..n)
                    .map(|s| -> Box<dyn Agent> {
                        if is_trader(s) {
                            Box::new(Timed {
                                inner: {
                                    let a = TraderAgent::new(seed ^ (s as u64 + 1));
                                    if passive { a.passive() } else { a }
                                },
                                nanos: nanos.clone(),
                                calls: calls.clone(),
                            })
                        } else if other == "random" {
                            Box::new(RandomAgent::new(seed ^ (s as u64 + 100)))
                        } else {
                            Box::new(GreedyAgent::new(seed ^ (s as u64 + 100)))
                        }
                    })
                    .collect();
                let mut steps = 0u32;
                while let Some(p) = game.current_player() {
                    let legal = game.legal_actions();
                    let a = agents[p.0].choose_action(&game, &legal);
                    game.apply(a).expect("legal action");
                    steps += 1;
                    assert!(steps < 50_000, "game did not terminate");
                }
                let rank = game.ranking();
                let best = game.player(rank[0]).best_score;
                // Ties for first share the win.
                let tied: Vec<_> = rank.iter().filter(|&&p| game.player(p).best_score == best).collect();
                for &&p in &tied {
                    if is_trader(p.0) {
                        tally.wins += 1.0 / tied.len() as f64;
                    }
                }
                tally.games += 1;
                tally.trader_seats += traders as u64;
                tally.trades += game.history().iter().filter(|a| matches!(a, Action::AcceptTrade)).count() as u64;
                tally.nanos += nanos.load(Ordering::Relaxed);
                tally.calls += calls.load(Ordering::Relaxed);
                g += threads as u64;
            }
            tally
        }));
    }
    let mut total = Tally::default();
    for h in handles {
        let t = h.join().unwrap();
        total.games += t.games;
        total.trader_seats += t.trader_seats;
        total.wins += t.wins;
        total.trades += t.trades;
        total.nanos += t.nanos;
        total.calls += t.calls;
    }
    total
}

fn main() {
    let games: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let passive = std::env::args().nth(2).is_some_and(|s| s == "passive");
    if passive {
        println!("(trader does not propose or counter)");
    }
    let config = Arc::new(GameConfig::embedded());
    let mut all_nanos = 0u64;
    let mut all_calls = 0u64;
    println!("{games} games per row; fair share is 1/players per trader seat");
    let quick = std::env::args().nth(3).is_some_and(|s| s == "quick");
    let rows: &[(usize, &str)] = if quick {
        &[(1, "greedy")]
    } else {
        &[(1, "greedy"), (1, "random"), (2, "greedy"), (3, "greedy")]
    };
    for &(traders, other) in rows {
        for n in [3usize, 4, 5] {
            if traders >= n {
                continue;
            }
            let t = run(&config, n, traders, other, games, 1000, passive);
            all_nanos += t.nanos;
            all_calls += t.calls;
            let rate = t.wins / t.games as f64;
            let per_seat = rate / traders as f64;
            println!(
                "{n}p  {traders} trader(s) vs {:>6}: trader win rate/seat {:5.1}% (fair {:4.1}%)  trades/game {:4.2}  {:.3} ms/decision",
                other,
                per_seat * 100.0,
                100.0 / n as f64,
                t.trades as f64 / t.games as f64,
                t.nanos as f64 / t.calls.max(1) as f64 / 1e6,
            );
        }
    }
    // All traders: negotiations between traders must terminate.
    for n in [3usize, 4, 5].into_iter().filter(|_| !quick) {
        let t = run(&config, n, n, "greedy", games.min(300), 5000, passive);
        all_nanos += t.nanos;
        all_calls += t.calls;
        println!(
            "{n}p  all traders: trades/game {:4.2}  {:.3} ms/decision",
            t.trades as f64 / t.games as f64,
            t.nanos as f64 / t.calls.max(1) as f64 / 1e6
        );
    }
    for (i, name) in ["invest", "acquire", "trade", "produce", "close"].iter().enumerate() {
        let c = PH_CALLS[i].load(Ordering::Relaxed).max(1);
        println!("  {name:8} {:>9} calls  {:.3} ms/decision", c, PH_NANOS[i].load(Ordering::Relaxed) as f64 / c as f64 / 1e6);
    }
    println!("overall {:.3} ms/decision", all_nanos as f64 / all_calls.max(1) as f64 / 1e6);
}
