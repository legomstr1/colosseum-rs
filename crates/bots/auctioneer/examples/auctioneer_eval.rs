//! Plays the auctioneer against GreedyAgent and RandomAgent.
//! `cargo run --release -p colosseum-bot-auctioneer --example auctioneer_eval [games_per_cell]`

use colosseum_bot_auctioneer::{AuctioneerAgent, Params};
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::{Action, Game, GameConfig};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Timed {
    inner: AuctioneerAgent,
    spent: Duration,
    calls: u64,
}

impl Agent for Timed {
    fn name(&self) -> String {
        self.inner.name()
    }
    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let t = Instant::now();
        let a = self.inner.choose_action(game, legal);
        if std::env::var("LOG").is_ok() && !matches!(a, Action::Bid { .. } | Action::PassBid | Action::EndTrading | Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }) {
            let me = game.current_player().unwrap();
            let q = game.player(me);
            println!("r{} {:?} coins {} med {} -> {:?} | mk {:?}", game.round(), std::mem::discriminant(game.phase()), q.coins, q.medals, a, game.markets());
        }
        self.spent += t.elapsed();
        self.calls += 1;
        a
    }
}

#[derive(Default, Clone, Copy)]
struct Tally {
    games: u64,
    wins: f64,
    spent: Duration,
    calls: u64,
}

/// Parameter overrides from the environment, e.g. `P_deny=0.4`.
fn params() -> Params {
    let mut p = Params::default();
    let get = |k: &str| std::env::var(format!("P_{k}")).ok().and_then(|v| v.parse::<f64>().ok());
    if let Some(v) = get("gain_per_round") { p.gain_per_round = v; }
    if let Some(v) = get("second_weight") { p.second_weight = v; }
    if let Some(v) = get("arena_pen") { p.arena_pen = v; }
    if let Some(v) = get("lambda_first") { p.lambda_first = v; }
    if let Some(v) = get("lambda_last") { p.lambda_last = v; }
    if let Some(v) = get("deny") { p.deny = v; }
    if let Some(v) = get("loge_pts") { p.loge_pts = v; }
    if let Some(v) = get("alt_frac") { p.alt_frac = v; }
    if let Some(v) = get("wealth") { p.wealth = v; }
    if let Some(v) = get("tail") { p.tail = v; }
    if let Some(v) = get("invest_min") { p.invest_min = v; }
    if let Some(v) = get("extra_min") { p.extra_min = v; }
    if let Some(v) = get("medal_extra_min") { p.medal_extra_min = v; }
    if let Some(v) = get("max_offers") { p.max_offers = v as u32; }
    if let Some(v) = get("reserve") { p.reserve = v as u32; }
    p
}

fn run(config: &Arc<GameConfig>, n: usize, seat: usize, opp: &str, seed: u64) -> (bool, f64, Duration, u64) {
    let mut game = Game::new(config.clone(), n, seed).unwrap();
    let mut me = Timed { inner: AuctioneerAgent::with_params(params()), spent: Duration::ZERO, calls: 0 };
    let mut others: Vec<Box<dyn Agent>> = (0..n)
        .map(|i| match opp {
            "greedy" => Box::new(GreedyAgent::new(seed * 31 + i as u64)) as Box<dyn Agent>,
            _ => Box::new(RandomAgent::new(seed * 31 + i as u64)),
        })
        .collect();
    let mut guard = 0u64;
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let action = if p.0 == seat {
            me.choose_action(&game, &legal)
        } else {
            others[p.0].choose_action(&game, &legal)
        };
        game.apply(action).expect("legal action");
        guard += 1;
        assert!(guard < 200_000);
    }
    if std::env::var("DIAG").is_ok() {
        let f = |q: &colosseum_engine::PlayerState| format!("best {:3} prog {} arena {} tix {} loge {} coins {:3} med {}", q.best_score, q.programs.len(), q.arena.len, q.season_tickets, q.loge as u8, q.coins, q.medals);
        let o: Vec<String> = game.players().iter().filter(|q| q.id.0 != seat).map(f).collect();
        println!("DIAG me[{}] others[{}]", f(game.player(colosseum_engine::PlayerId(seat))), o.join(" | "));
    }
    let ranking = game.ranking();
    let top = game.player(ranking[0]);
    let tied = game
        .players()
        .iter()
        .filter(|q| q.best_score == top.best_score && q.coins == top.coins)
        .count();
    let won = ranking[0].0 == seat;
    (won, if won { 1.0 / tied as f64 } else { 0.0 }, me.spent, me.calls)
}

fn main() {
    let per_cell: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(400);
    let config = Arc::new(GameConfig::embedded());
    println!("auctioneer vs opponents; baseline win rate is 1/n");
    let only = std::env::args().nth(2);
    for opp in ["greedy", "random"] {
        if only.as_deref().is_some_and(|o| o != opp) { continue; }
        for n in 3..=5usize {
            let mut results = Vec::new();
            std::thread::scope(|s| {
                let handles: Vec<_> = (0..n)
                    .map(|seat| {
                        let config = config.clone();
                        s.spawn(move || {
                            let mut t = Tally::default();
                            for g in 0..per_cell / n as u64 {
                                let (_, w, sp, c) = run(&config, n, seat, opp, 1000 + g * 7 + seat as u64);
                                t.games += 1;
                                t.wins += w;
                                t.spent += sp;
                                t.calls += c;
                            }
                            t
                        })
                    })
                    .collect();
                for h in handles {
                    results.push(h.join().unwrap());
                }
            });
            let games: u64 = results.iter().map(|t| t.games).sum();
            let wins: f64 = results.iter().map(|t| t.wins).sum();
            let spent: Duration = results.iter().map(|t| t.spent).sum();
            let calls: u64 = results.iter().map(|t| t.calls).sum();
            let per_seat: Vec<String> = results.iter().map(|t| format!("{:.0}%", 100.0 * t.wins / t.games as f64)).collect();
            println!(
                "{n}p vs {opp:6}: win {:5.1}% (base {:4.1}%)  games {games}  ms/decision {:.4}  seats [{}]",
                100.0 * wins / games as f64,
                100.0 / n as f64,
                spent.as_secs_f64() * 1000.0 / calls as f64,
                per_seat.join(" ")
            );
        }
    }
}
