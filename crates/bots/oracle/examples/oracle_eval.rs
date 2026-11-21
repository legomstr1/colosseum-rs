//! Plays the oracle against planner, auctioneer and trader.
//!
//! `cargo run --release -p colosseum-bot-oracle --example oracle_eval -- [games] [section] [key=value ...]`
//!
//! Sections: `all` (default), `mixed`, `h2h`, `time`. Seats are drawn with
//! repeats; a game always seats at least one oracle (and, head to head, at
//! least one opponent). Win rate is reported against fair share: wins divided
//! by the sum of 1/players over the oracle's seats. Timing runs on one thread.

use colosseum_bot_auctioneer::AuctioneerAgent;
use colosseum_bot_oracle::{OracleAgent, Params};
use colosseum_bot_planner::PlannerAgent;
use colosseum_bot_trader::TraderAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const OPPS: [&str; 3] = ["planner", "auctioneer", "trader"];

fn set(p: &mut Params, k: &str, v: f64) {
    match k {
        "gain_rate" => p.gain_rate = v,
        "trade_rate" => p.trade_rate = v,
        "token_cost" => p.token_cost = v,
        "income" => p.income = v,
        "l1" => p.lambda[0] = v,
        "l2" => p.lambda[1] = v,
        "l3" => p.lambda[2] = v,
        "l4" => p.lambda[3] = v,
        "l5" => p.lambda[4] = v,
        "lscale" => p.lambda.iter_mut().for_each(|x| *x *= v),
        "lambda_end" => p.lambda_end = v,
        "lambda_final_acq" => p.lambda_final_acq = v,
        "ticket_disc" => p.ticket_disc = v,
        "noble_base" => p.noble_base = v,
        "noble_sq" => p.noble_sq = v,
        "noble_loge" => p.noble_loge = v,
        "loge_round" => p.loge_round = v,
        "second" => p.second = v,
        "se_value" => p.se_value = v,
        "best_w" => p.best_w = v,
        "medal_value" => p.medal_value = v,
        "invest_min" => p.invest_min = v,
        "trade_k" => p.trade_k = v,
        "accept_margin" => p.accept_margin = v,
        "buy_min" => p.buy_min = v,
        "max_offers" => p.max_offers = v as u32,
        "pay_frac" => p.pay_frac = v,
        "deny" => p.deny = v,
        "alt_frac" => p.alt_frac = v,
        "bid_frac" => p.bid_frac = v,
        "tau" => p.tau = v,
        "opp_extra" => p.opp_extra = v,
        "step_penalty" => p.step_penalty = v,
        "income_w" => p.income_w = v,
        "token_w" => p.token_w = v,
        "joker_w" => p.joker_w = v,
        "podium_w" => p.podium_w = v,
        "podium_tau" => p.podium_tau = v,
        "open_frac" => p.open_frac = v,
        "reserve" => p.reserve = v,
        "reserve_mult" => p.reserve_mult = v,
        "smart_open" => p.smart_open = v,
        "probe_tries" => p.probe_tries = v as u32,
        "use_floor" => p.use_floor = v,
        "swaps" => p.swaps = v,
        "swap_min" => p.swap_min = v,
        "threat_tau" => p.threat_tau = v,
        "final_wp" => p.final_wp = v,
        _ => panic!("unknown param {k}"),
    }
}

fn make(name: &str, seed: u64, params: &Params) -> Box<dyn Agent> {
    match name {
        "oracle" => Box::new(OracleAgent::with_params(params.clone())),
        "planner" => Box::new(PlannerAgent::new(seed)),
        "auctioneer" => Box::new(AuctioneerAgent::new(seed)),
        "trader" => Box::new(TraderAgent::new(seed)),
        _ => unreachable!(),
    }
}

struct Outcome {
    n: usize,
    seats: Vec<&'static str>,
    winner: usize,
    wall: Duration,
    oracle_think: Duration,
    oracle_decisions: u64,
    /// Actions per phase kind (investing..closing, other).
    phase_actions: [u64; 6],
    phase_time: [Duration; 6],
}

fn play(config: &Arc<GameConfig>, seats: Vec<&'static str>, seed: u64, params: &Params) -> Outcome {
    let n = seats.len();
    let mut game = Game::new(config.clone(), n, seed).expect("game");
    let mut agents: Vec<Box<dyn Agent>> = seats
        .iter()
        .enumerate()
        .map(|(i, s)| make(s, seed.wrapping_mul(31).wrapping_add(i as u64), params))
        .collect();
    let mut think = Duration::ZERO;
    let mut decisions = 0u64;
    let start = Instant::now();
    let mut steps = 0u64;
    let mut phase_actions = [0u64; 6];
    let mut phase_time = [Duration::ZERO; 6];
    while let Some(p) = game.current_player() {
        let k = game.phase().kind().map_or(5, |k| k as usize);
        let t0 = Instant::now();
        let legal = game.legal_actions();
        let t = Instant::now();
        let a = agents[p.0].choose_action(&game, &legal);
        if seats[p.0] == "oracle" {
            think += t.elapsed();
            decisions += 1;
        }
        if let Err(e) = game.apply(a) {
            panic!("{} chose an illegal action: {e} (seed {seed}, seats {seats:?})", seats[p.0]);
        }
        phase_actions[k] += 1;
        phase_time[k] += t0.elapsed();
        steps += 1;
        assert!(steps < 200_000, "game did not end (seed {seed})");
    }
    let wall = start.elapsed();
    Outcome {
        n,
        winner: game.ranking()[0].0,
        seats,
        wall,
        oracle_think: think,
        oracle_decisions: decisions,
        phase_actions,
        phase_time,
    }
}

/// Seats for game `g`: drawn with repeats from `pool`, at least one oracle and
/// (if `need` is set) at least one of that opponent.
fn draw(g: u64, n: usize, pool: &[&'static str], need: Option<&'static str>) -> Vec<&'static str> {
    let mut rng = Rng::new(g.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x0AC1E);
    loop {
        let seats: Vec<&'static str> = (0..n).map(|_| pool[rng.below(pool.len())]).collect();
        if seats.contains(&"oracle") && need.is_none_or(|x| seats.contains(&x)) {
            return seats;
        }
    }
}

#[derive(Default, Clone)]
struct Tally {
    games: u64,
    wins: f64,
    fair: f64,
}

impl Tally {
    fn add(&mut self, o: &Outcome) {
        self.games += 1;
        for s in &o.seats {
            if *s == "oracle" {
                self.fair += 1.0 / o.n as f64;
            }
        }
        if o.seats[o.winner] == "oracle" {
            self.wins += 1.0;
        }
    }
    fn ratio(&self) -> f64 {
        self.wins / self.fair.max(1e-9)
    }
}

/// Plays `games` games per player count in parallel and tallies them.
fn run_parallel(
    config: &Arc<GameConfig>,
    games: u64,
    pool: &[&'static str],
    need: Option<&'static str>,
    params: &Params,
    salt: u64,
) -> Vec<(usize, Tally, Vec<Tally>)> {
    let threads = std::thread::available_parallelism().map_or(4, |x| x.get());
    let mut out = Vec::new();
    for n in 3..=5usize {
        let next = AtomicU64::new(0);
        let results: Vec<Outcome> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..threads)
                .map(|_| {
                    s.spawn(|| {
                        let mut v = Vec::new();
                        loop {
                            let g = next.fetch_add(1, Ordering::Relaxed);
                            if g >= games {
                                break;
                            }
                            let id = salt.wrapping_mul(1_000_003).wrapping_add(g * 10 + n as u64);
                            let seats = draw(id, n, pool, need);
                            v.push(play(config, seats, id, params));
                        }
                        v
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().expect("worker")).collect()
        });
        let mut all = Tally::default();
        // Per opponent: games where both the oracle and that opponent sat.
        let mut per = vec![Tally::default(); OPPS.len()];
        for o in &results {
            all.add(o);
            for (k, opp) in OPPS.iter().enumerate() {
                if o.seats.contains(opp) {
                    per[k].add(o);
                }
            }
        }
        out.push((n, all, per));
    }
    out
}

/// `kind`: 0 = all oracle, 1 = oracle mixed with the others, 2 = reference
/// (the other three bots only, no oracle).
fn timing(config: &Arc<GameConfig>, games: u64, kind: u32, params: &Params) -> (f64, f64, f64, f64) {
    let pool: Vec<&'static str> = match kind {
        0 => vec!["oracle"],
        1 => vec!["oracle", "planner", "auctioneer", "trader"],
        _ => vec!["planner", "auctioneer", "trader"],
    };
    let mut walls = Vec::new();
    let mut think = Duration::ZERO;
    let mut dec = 0u64;
    let mut pa = [0u64; 6];
    let mut pt = [Duration::ZERO; 6];
    for g in 0..games {
        let id = 77_000 + g;
        let seats = if kind == 2 {
            let mut rng = Rng::new(id);
            (0..5).map(|_| pool[rng.below(pool.len())]).collect()
        } else {
            draw(id, 5, &pool, None)
        };
        let o = play(config, seats, id, params);
        walls.push(o.wall.as_secs_f64() * 1000.0);
        think += o.oracle_think;
        dec += o.oracle_decisions;
        for i in 0..6 {
            pa[i] += o.phase_actions[i];
            pt[i] += o.phase_time[i];
        }
    }
    if std::env::var_os("ORACLE_PHASES").is_some() {
        for (i, name) in ["invest", "acquire", "trade", "produce", "close", "other"].iter().enumerate() {
            println!(
                "      {name:8} {:7.1} actions/game {:6.2} ms/game",
                pa[i] as f64 / games as f64,
                pt[i].as_secs_f64() * 1000.0 / games as f64
            );
        }
    }
    walls.sort_by(f64::total_cmp);
    let mean = walls.iter().sum::<f64>() / walls.len() as f64;
    let p95 = walls[((walls.len() as f64 * 0.95) as usize).min(walls.len() - 1)];
    let ms_dec = think.as_secs_f64() * 1000.0 / dec.max(1) as f64;
    let dec_game = dec as f64 / games as f64;
    (mean, p95, ms_dec, dec_game)
}

fn main() {
    let mut params = Params::default();
    let mut pos = Vec::new();
    for a in std::env::args().skip(1) {
        if let Some((k, v)) = a.split_once('=') {
            set(&mut params, k, v.parse().expect("number"));
        } else {
            pos.push(a);
        }
    }
    let games: u64 = pos.first().and_then(|s| s.parse().ok()).unwrap_or(2000);
    let section = pos.get(1).cloned().unwrap_or_else(|| "all".into());
    let salt: u64 = pos.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let config = Arc::new(GameConfig::embedded());

    if section == "obj" {
        // Tuning objective: the mixed ratio averaged with the weakest
        // head-to-head ratio.
        let total = |res: &[(usize, Tally, Vec<Tally>)]| {
            let mut t = Tally::default();
            for (_, a, _) in res {
                t.wins += a.wins;
                t.fair += a.fair;
            }
            t.ratio()
        };
        let mixed = total(&run_parallel(&config, games, &["oracle", "planner", "auctioneer", "trader"], None, &params, salt));
        let mut worst = f64::INFINITY;
        for opp in OPPS {
            let r = total(&run_parallel(&config, games / 2, &["oracle", opp], Some(opp), &params, salt + 17));
            print!("{opp} {r:.3}  ");
            worst = worst.min(r);
        }
        println!("mixed {mixed:.3}");
        println!("  all  oracle {:.3}x fair (objective)", 0.5 * (mixed + worst));
        return;
    }
    if section == "all" || section == "mixed" {
        println!("== Mixed: seats drawn from oracle/planner/auctioneer/trader ({games} games per size)");
        let res = run_parallel(&config, games, &["oracle", "planner", "auctioneer", "trader"], None, &params, salt);
        let mut tot = Tally::default();
        for (n, all, per) in &res {
            tot.games += all.games;
            tot.wins += all.wins;
            tot.fair += all.fair;
            print!("  {n}p  oracle {:.3}x fair", all.ratio());
            for (k, t) in per.iter().enumerate() {
                print!("  | with {} {:.3}x", OPPS[k], t.ratio());
            }
            println!();
        }
        println!("  all  oracle {:.3}x fair", tot.ratio());
    }
    if section == "all" || section == "h2h" {
        println!("== Head to head: seats drawn from oracle + one opponent ({games} games per size)");
        let mut grand = Tally::default();
        for opp in OPPS {
            let res = run_parallel(&config, games, &["oracle", opp], Some(opp), &params, salt + 17);
            let mut tot = Tally::default();
            print!("  vs {opp:<10}");
            for (n, all, _) in &res {
                tot.games += all.games;
                tot.wins += all.wins;
                tot.fair += all.fair;
                print!("  {n}p {:.3}x", all.ratio());
            }
            println!("  | all {:.3}x", tot.ratio());
            grand.games += tot.games;
            grand.wins += tot.wins;
            grand.fair += tot.fair;
        }
        println!("  h2h overall {:.3}x", grand.ratio());
    }
    if section == "all" || section == "time" {
        let tg = games.clamp(50, 500);
        println!("== Timing: 5-player games on one thread ({tg} games each)");
        for kind in [0, 1, 2] {
            let (mean, p95, ms_dec, dec) = timing(&config, tg, kind, &params);
            let label = ["all-oracle", "mixed", "reference"][kind as usize];
            if kind == 2 {
                println!("  {label:<10} mean {mean:6.2} ms  p95 {p95:6.2} ms  (planner/auctioneer/trader only, for calibration)");
            } else {
                println!("  {label:<10} mean {mean:6.2} ms  p95 {p95:6.2} ms  oracle {ms_dec:.4} ms/decision ({dec:.0} decisions/game)");
            }
        }
    }
}
