//! Sweep harness: plays each variant on the same game seeds (common random
//! numbers) against a fixed field and reports win rate, placement and score.
//!
//! cargo run --release -p colosseum-bot-live-input --example sweep -- \
//!     --games 4000 --sizes 4 --field house --seats 1 \
//!     --variants "house;core;core{plan=TEPE}"
//!
//! Agent specs: `house`, `search`, `core{k=v,...}` (heuristic with param
//! overrides), `t1{k=v,...}` (tuned-1), `t2{k=v,...}` (tuned-2), `ie{k=v,...}` (int-event), `li{k=v,...,s.budget=96,s.rollout=core}` (search over the
//! heuristic). `--field` is a list of specs separated by `|`; opponents are
//! drawn from it per seat.

use colosseum_bot_live_input::{CoreAgent, LiveInputAgent, Params, SearchConfig};
use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Game, GameConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn split_spec(spec: &str) -> (&str, &str) {
    match spec.find('{') {
        Some(i) => (&spec[..i], spec[i + 1..].trim_end_matches('}')),
        None => (spec, ""),
    }
}

fn make(spec: &str, seed: u64) -> Box<dyn Agent> {
    let (kind, over) = split_spec(spec.trim());
    let (s_over, p_over): (Vec<&str>, Vec<&str>) = over.split(',').filter(|s| !s.is_empty()).partition(|s| s.starts_with("s."));
    let params = || Params::default().with_overrides(&p_over.join(",")).unwrap_or_else(|e| panic!("{spec}: {e}"));
    match kind {
        "house" => Box::new(colosseum_bot_house::HouseAgent::new(seed)),
        "search" => Box::new(colosseum_bot_search::SearchAgent::new(seed)),
        "oracle" => Box::new(colosseum_bot_oracle::OracleAgent::new(seed)),
        "maestro" => Box::new(colosseum_bot_maestro::MaestroAgent::new(seed)),
        "chameleon" => Box::new(colosseum_bot_chameleon::ChameleonAgent::new(seed)),
        "trader" => Box::new(colosseum_bot_trader::TraderAgent::new(seed)),
        "planner" => Box::new(colosseum_bot_planner::PlannerAgent::new(seed)),
        "showman" => Box::new(colosseum_bot_showman::ShowmanAgent::new(seed)),
        "auctioneer" => Box::new(colosseum_bot_auctioneer::AuctioneerAgent::new(seed)),
        "core" => Box::new(CoreAgent::with_params(seed, params())),
        "t1" => {
            let p = colosseum_bot_tuned_1::Params::default().with_overrides(over).unwrap_or_else(|e| panic!("{spec}: {e}"));
            Box::new(colosseum_bot_tuned_1::Tuned1Agent::with_params(seed, p))
        }
        "t2" => {
            let p = colosseum_bot_tuned_2::Params::default().with_overrides(over).unwrap_or_else(|e| panic!("{spec}: {e}"));
            Box::new(colosseum_bot_tuned_2::Tuned2Agent::with_params(seed, p))
        }
        "ie" => {
            let p = colosseum_bot_int_event::Params::default().with_overrides(over).unwrap_or_else(|e| panic!("{spec}: {e}"));
            Box::new(colosseum_bot_int_event::IntEventAgent::with_params(seed, p))
        }
        "li" => {
            let mut cfg = SearchConfig::default();
            for kv in s_over {
                let (k, v) = kv[2..].split_once('=').expect("s.key=value");
                match k {
                    "budget" => cfg.budget = v.parse().unwrap(),
                    "rollout" => cfg.rollout = v.into(),
                    "early_boost" => cfg.early_boost = v.parse().unwrap(),
                    "max_candidates" => cfg.max_candidates = v.parse().unwrap(),
                    "base_bonus" => cfg.base_bonus = v.parse().unwrap(),
                    "margin_weight" => cfg.margin_weight = v.parse().unwrap(),
                    "phases" => cfg.phases = v.into(),
                    _ => panic!("unknown search key {k}"),
                }
            }
            Box::new(LiveInputAgent::with_config(seed, params(), cfg))
        }
        _ => panic!("unknown agent {spec}"),
    }
}

static DONE: AtomicUsize = AtomicUsize::new(0);
static TRADES: AtomicUsize = AtomicUsize::new(0);
static MONEY: AtomicUsize = AtomicUsize::new(0);
static COINS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default, Clone)]
struct Stat {
    n: f64,
    wins: f64,
    place: f64,
    score: f64,
    field_score: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let games: usize = get("--games").map_or(2000, |s| s.parse().unwrap());
    let sizes: Vec<usize> = get("--sizes").unwrap_or("4".into()).split(',').map(|s| s.parse().unwrap()).collect();
    let field: Vec<String> = get("--field").unwrap_or("house".into()).split('|').map(String::from).collect();
    let seats: usize = get("--seats").map_or(1, |s| s.parse().unwrap());
    let seed0: u64 = get("--seed").map_or(1000, |s| s.parse().unwrap());
    let threads: usize = get("--threads").map_or(16, |s| s.parse().unwrap());
    let variants: Vec<String> = get("--variants").unwrap_or("house;core".into()).split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

    let config = Arc::new(GameConfig::embedded());
    let jobs: Vec<(usize, usize)> = (0..variants.len()).flat_map(|v| (0..games).map(move |g| (v, g))).collect();
    let next = AtomicUsize::new(0);
    let stats = Mutex::new(vec![Stat::default(); variants.len()]);
    let start = std::time::Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let j = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(v, g)) = jobs.get(j) else { break };
                let gseed = seed0 + g as u64;
                let mut rng = Rng::new(gseed ^ 0x005E_ED0F_5EA7);
                let n = sizes[rng.below(sizes.len())];
                // Candidate seats: a rotating block, so every seat position is covered.
                let first = rng.below(n);
                let mine: Vec<usize> = (0..seats.min(n)).map(|k| (first + k) % n).collect();
                let specs: Vec<String> = (0..n)
                    .map(|i| if mine.contains(&i) { variants[v].clone() } else { field[rng.below(field.len())].clone() })
                    .collect();
                let mut game = Game::new(config.clone(), n, gseed).unwrap();
                let mut agents: Vec<Box<dyn Agent>> =
                    specs.iter().enumerate().map(|(i, sp)| make(sp, gseed.wrapping_mul(31).wrapping_add(i as u64))).collect();
                let mut steps = 0;
                while let Some(p) = game.current_player() {
                    let legal = game.legal_actions();
                    let a = agents[p.0].choose_action(&game, &legal);
                    if a == colosseum_engine::Action::AcceptTrade
                        && let colosseum_engine::Phase::Trading { offer: Some(o), .. } = game.phase()
                    {
                        let money = o.terms.give_coins > 0 || o.terms.receive_coins > 0;
                        TRADES.fetch_add(1, Ordering::Relaxed);
                        if money {
                            MONEY.fetch_add(1, Ordering::Relaxed);
                            COINS.fetch_add((o.terms.give_coins + o.terms.receive_coins) as usize, Ordering::Relaxed);
                        }
                    }
                    if let Err(e) = game.apply(a.clone()) {
                        panic!("{} chose illegal {a:?}: {e:?} (seed {gseed})", specs[p.0]);
                    }
                    steps += 1;
                    assert!(steps < 200_000, "runaway game seed {gseed}");
                }
                let ranking = game.ranking();
                let others: Vec<f64> =
                    (0..n).filter(|i| !mine.contains(i)).map(|i| game.players()[i].best_score as f64).collect();
                let field_mean = others.iter().sum::<f64>() / others.len().max(1) as f64;
                let done = DONE.fetch_add(1, Ordering::Relaxed) + 1;
                if done.is_multiple_of(20) {
                    eprintln!("  {done} / {} games ({:.0}s)", jobs.len(), start.elapsed().as_secs_f64());
                }
                let mut st = stats.lock().unwrap();
                for &i in &mine {
                    let pos = ranking.iter().position(|p| p.0 == i).unwrap();
                    let e = &mut st[v];
                    e.n += 1.0;
                    e.wins += (pos == 0) as u8 as f64;
                    e.place += 1.0 - pos as f64 / (n - 1) as f64;
                    e.score += game.players()[i].best_score as f64;
                    e.field_score += field_mean;
                }
            });
        }
    });
    let st = stats.into_inner().unwrap();
    println!(
        "{} games x {} variants, sizes {:?}, field {:?}, {} seat(s), {:.0}s",
        games,
        variants.len(),
        sizes,
        field,
        seats,
        start.elapsed().as_secs_f64()
    );
    let (t, m, c) = (TRADES.load(Ordering::Relaxed), MONEY.load(Ordering::Relaxed), COINS.load(Ordering::Relaxed));
    println!("trades accepted: {t} ({:.2}/game), with money: {m} (avg {:.1} coins)", t as f64 / jobs.len() as f64, c as f64 / m.max(1) as f64);
    println!("{:>7} {:>6} {:>6} {:>7} {:>7}  variant", "win%", "±se", "place", "score", "field");
    for (v, e) in st.iter().enumerate() {
        let w = e.wins / e.n;
        let se = (w * (1.0 - w) / e.n).sqrt();
        println!(
            "{:>6.2}% {:>5.2}% {:>6.3} {:>7.2} {:>7.2}  {}",
            100.0 * w,
            100.0 * se,
            e.place / e.n,
            e.score / e.n,
            e.field_score / e.n,
            variants[v]
        );
    }
}
