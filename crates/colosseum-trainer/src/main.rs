//! Unattended self-play tuner for the Oracle bot's `Params`.
//!
//! Each generation:
//! 1. Draws `pop` mutants of the champion (a random subset of numeric
//!    parameters scaled by `exp(sigma * N(0,1))`).
//! 2. Plays every mutant, and the champion itself, in the same seat of the
//!    same `games` games (common random numbers), against a league of fixed
//!    bots, the champion, and past champions ("hall").
//! 3. Takes the mutant with the best paired gain over the champion and
//!    re-tests it on `confirm` fresh games. It becomes the new champion only
//!    if that gain is more than `z` standard errors above zero.
//!
//! Everything lives in `--dir`: `state.json` (resumable; the run can be
//! stopped at any time), `champion.json` (the best `Params`, loadable by the
//! tournament as `oracle-tuned`) and `log.csv` (one row per generation).

use clap::Parser;
use colosseum_bot_oracle::{OracleAgent, Params};
use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Game, GameConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "train", about = "Tune Oracle's parameters by self-play, unattended")]
struct Cli {
    /// Where state, champion and log are kept.
    #[arg(long, default_value = "tuning/oracle")]
    dir: PathBuf,
    /// League the candidate plays against. `champion` is the current best
    /// params; `hall` is a random past champion.
    #[arg(long, value_delimiter = ',', default_value = "house,house,house,champion,hall")]
    opponents: Vec<String>,
    /// Mutants per generation.
    #[arg(long, default_value_t = 8)]
    pop: usize,
    /// Screening games per mutant per generation.
    #[arg(long, default_value_t = 1000)]
    games: usize,
    /// Fresh games used to confirm the best mutant.
    #[arg(long, default_value_t = 4000)]
    confirm: usize,
    /// Standard errors the confirmed gain must clear.
    #[arg(long, default_value_t = 2.0)]
    z: f64,
    /// Stop after this many generations (default: run until killed).
    #[arg(long)]
    generations: Option<u64>,
    /// Initial mutation size (only used when starting fresh).
    #[arg(long, default_value_t = 0.25)]
    sigma: f64,
    /// Share of parameters mutated in each mutant.
    #[arg(long, default_value_t = 0.25)]
    mutate_frac: f64,
    /// Table sizes to draw from.
    #[arg(long, value_delimiter = ',', default_value = "3,4,5")]
    sizes: Vec<usize>,
    /// Fitness: `win`, `place` (0 last .. 1 first) or `mix` (their mean).
    #[arg(long, default_value = "mix")]
    metric: String,
    #[arg(long)]
    threads: Option<usize>,
}

#[derive(Serialize, Deserialize)]
struct State {
    generation: u64,
    sigma: f64,
    champion: Value,
    /// Past champions, newest last.
    hall: Vec<Value>,
    promotions: u64,
}

const HALL_SIZE: usize = 8;

fn to_params(v: &Value) -> Params {
    serde_json::from_value(v.clone()).expect("params round-trip")
}

fn make_bot(name: &str, seed: u64, champ: &Params, hall: &[Params], rng: &mut Rng) -> Box<dyn Agent> {
    match name {
        "champion" => Box::new(OracleAgent::with_params(champ.clone())),
        "hall" => match rng.choose(hall) {
            Some(p) => Box::new(OracleAgent::with_params(p.clone())),
            None => Box::new(OracleAgent::with_params(champ.clone())),
        },
        "oracle" => Box::new(OracleAgent::new(seed)),
        "maestro" => Box::new(colosseum_bot_maestro::MaestroAgent::new(seed)),
        "chameleon" => Box::new(colosseum_bot_chameleon::ChameleonAgent::new(seed)),
        "house" => Box::new(colosseum_bot_house::HouseAgent::new(seed)),
        other => panic!("unknown opponent '{other}'"),
    }
}

/// Standard normal draw (Box-Muller).
fn gauss(rng: &mut Rng) -> f64 {
    let u1 = ((rng.next_u64() >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
    let u2 = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

fn count_leaves(v: &Value) -> usize {
    match v {
        Value::Number(_) => 1,
        Value::Array(a) => a.iter().map(count_leaves).sum(),
        Value::Object(o) => o.values().map(count_leaves).sum(),
        _ => 0,
    }
}

/// Scales each numeric leaf with probability `frac`. Signs never flip;
/// integers stay non-negative integers; zeros get a small absolute nudge.
fn mutate(v: &mut Value, sigma: f64, frac: f64, rng: &mut Rng, changed: &mut usize) {
    match v {
        Value::Number(n) => {
            if (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64 >= frac {
                return;
            }
            *changed += 1;
            let step = sigma * gauss(rng);
            if n.is_u64() {
                let x = n.as_u64().unwrap() as f64;
                let y = if x == 0.0 { step.abs() * 2.0 } else { x * step.exp() };
                let mut y = y.round() as u64;
                if y == x as u64 {
                    y = if step > 0.0 { y + 1 } else { y.saturating_sub(1) };
                }
                *v = Value::from(y);
            } else {
                let x = n.as_f64().unwrap();
                let y = if x == 0.0 { step.abs() * 0.1 } else { x * step.exp() };
                *v = serde_json::Number::from_f64(y).map_or(Value::from(x), Value::Number);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| mutate(x, sigma, frac, rng, changed)),
        Value::Object(o) => o.values_mut().for_each(|x| mutate(x, sigma, frac, rng, changed)),
        _ => {}
    }
}

/// Plays one game with `cand` in a random seat; returns its fitness.
fn play(config: &Arc<GameConfig>, cli: &Cli, cand: &Params, champ: &Params, hall: &[Params], seed: u64) -> f64 {
    let mut rng = Rng::new(seed ^ 0x5EED_1234_ABCD_0001);
    let size = cli.sizes[rng.below(cli.sizes.len())];
    let seat = rng.below(size);
    let mut agents: Vec<Box<dyn Agent>> = (0..size)
        .map(|i| {
            let s = seed.wrapping_mul(31).wrapping_add(i as u64);
            if i == seat {
                Box::new(OracleAgent::with_params(cand.clone())) as Box<dyn Agent>
            } else {
                let name = cli.opponents[rng.below(cli.opponents.len())].clone();
                make_bot(&name, s, champ, hall, &mut rng)
            }
        })
        .collect();
    let mut game = Game::new(config.clone(), size, seed).expect("valid table size");
    let mut steps = 0u32;
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let action = agents[p.0].choose_action(&game, &legal);
        game.apply(action).expect("bots choose legal actions");
        steps += 1;
        if steps > 200_000 {
            return 0.0;
        }
    }
    let rank = game.ranking().iter().position(|p| p.0 == seat).unwrap();
    let win = if rank == 0 { 1.0 } else { 0.0 };
    let place = (size - 1 - rank) as f64 / (size - 1) as f64;
    match cli.metric.as_str() {
        "win" => win,
        "place" => place,
        _ => 0.5 * (win + place),
    }
}

/// Fitness of each candidate on games `seed_base..seed_base+n`, in parallel.
fn evaluate(config: &Arc<GameConfig>, cli: &Cli, cands: &[Params], champ: &Params, hall: &[Params], seed_base: u64, n: usize) -> Vec<Vec<f64>> {
    let threads = thread_count(cli);
    let out = Mutex::new(vec![vec![0.0; n]; cands.len()]);
    let next = AtomicUsize::new(0);
    let total = cands.len() * n;
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut local = Vec::new();
                loop {
                    let j = next.fetch_add(1, Ordering::Relaxed);
                    if j >= total {
                        break;
                    }
                    let (k, i) = (j % cands.len(), j / cands.len());
                    local.push((k, i, play(config, cli, &cands[k], champ, hall, seed_base + i as u64)));
                }
                let mut out = out.lock().unwrap();
                for (k, i, f) in local {
                    out[k][i] = f;
                }
            });
        }
    });
    out.into_inner().unwrap()
}

/// Mean and standard error of the paired difference a - b.
fn paired(a: &[f64], b: &[f64]) -> (f64, f64) {
    let n = a.len() as f64;
    let d: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
    let mean = d.iter().sum::<f64>() / n;
    let var = d.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (mean, (var / n).sqrt())
}

fn mean(a: &[f64]) -> f64 {
    a.iter().sum::<f64>() / a.len() as f64
}

fn thread_count(cli: &Cli) -> usize {
    cli.threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()))
}

fn write_atomic(path: &Path, text: &str) {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).expect("write state");
    fs::rename(&tmp, path).expect("replace state");
}

fn main() {
    let cli = Cli::parse();
    let config = Arc::new(GameConfig::embedded());
    let threads = thread_count(&cli);
    fs::create_dir_all(&cli.dir).expect("create tuning dir");
    let state_path = cli.dir.join("state.json");
    let champ_path = cli.dir.join("champion.json");
    let log_path = cli.dir.join("log.csv");

    let mut state: State = match fs::read_to_string(&state_path) {
        Ok(s) => serde_json::from_str(&s).expect("parse state.json"),
        Err(_) => State {
            generation: 0,
            sigma: cli.sigma,
            champion: serde_json::to_value(Params::default()).unwrap(),
            hall: Vec::new(),
            promotions: 0,
        },
    };
    // Pick up parameters added to `Params` since the state was saved.
    state.champion = serde_json::to_value(to_params(&state.champion)).unwrap();
    if !log_path.exists() {
        fs::write(&log_path, "generation,sigma,screen_gain,screen_se,confirm_gain,confirm_se,promoted,champ_fitness,changed,secs\n").unwrap();
    }
    let leaves = count_leaves(&state.champion);
    println!("tuning {leaves} parameters in {} on {threads} threads (generation {}, {} promotions so far)", cli.dir.display(), state.generation, state.promotions);

    let stop_at = cli.generations.map(|g| state.generation + g);
    while stop_at.is_none_or(|g| state.generation < g) {
        let t0 = Instant::now();
        let generation = state.generation;
        let mut rng = Rng::new(0xC0FFEE ^ generation.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let champ = to_params(&state.champion);
        let hall: Vec<Params> = state.hall.iter().map(to_params).collect();

        // Index 0 is the champion itself; the rest are mutants.
        let mut values = vec![state.champion.clone()];
        let mut changed = vec![0];
        for _ in 0..cli.pop {
            let mut v = state.champion.clone();
            let mut c = 0;
            while c == 0 {
                v = state.champion.clone();
                mutate(&mut v, state.sigma, cli.mutate_frac, &mut rng, &mut c);
            }
            values.push(v);
            changed.push(c);
        }
        let cands: Vec<Params> = values.iter().map(to_params).collect();

        // Seeds never repeat across generations or between screen and confirm.
        let seed_base = generation * 1_000_000_000;
        let fit = evaluate(&config, &cli, &cands, &champ, &hall, seed_base, cli.games);
        let (best, (gain, se)) = (1..cands.len())
            .map(|k| (k, paired(&fit[k], &fit[0])))
            .max_by(|a, b| a.1.0.total_cmp(&b.1.0))
            .unwrap();

        let (mut cgain, mut cse, mut promoted) = (0.0, 0.0, false);
        if gain > 0.0 {
            let pair = [cands[0].clone(), cands[best].clone()];
            let cf = evaluate(&config, &cli, &pair, &champ, &hall, seed_base + 500_000_000, cli.confirm);
            (cgain, cse) = paired(&cf[1], &cf[0]);
            promoted = cgain > cli.z * cse;
        }

        let champ_fit = mean(&fit[0]);
        if promoted {
            state.hall.push(state.champion.clone());
            if state.hall.len() > HALL_SIZE {
                state.hall.remove(0);
            }
            state.champion = values[best].clone();
            state.promotions += 1;
            state.sigma = (state.sigma * 1.15).min(0.6);
            write_atomic(&champ_path, &serde_json::to_string_pretty(&state.champion).unwrap());
        } else {
            state.sigma = (state.sigma * 0.97).max(0.04);
        }
        state.generation += 1;
        write_atomic(&state_path, &serde_json::to_string_pretty(&state).unwrap());
        if !champ_path.exists() {
            write_atomic(&champ_path, &serde_json::to_string_pretty(&state.champion).unwrap());
        }

        let secs = t0.elapsed().as_secs_f64();
        let mut log = fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        writeln!(log, "{generation},{:.4},{gain:.4},{se:.4},{cgain:.4},{cse:.4},{promoted},{champ_fit:.4},{},{secs:.1}", state.sigma, changed[best]).unwrap();
        println!(
            "generation {generation:>4}  champ {champ_fit:.3}  best mutant {gain:+.4}±{se:.4}  confirm {cgain:+.4}±{cse:.4}  {}  sigma {:.3}  {secs:.0}s",
            if promoted { "PROMOTED" } else { "kept    " },
            state.sigma
        );
    }
}
