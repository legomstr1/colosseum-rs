//! Bot tournament and timing benchmark.
//!
//! Each game draws a player count (3-5 by default) and a random set of
//! distinct bots from the pool, seats them in random order, and plays the game
//! out. Games run in parallel across worker threads. The report gives each
//! bot's win rate, average finishing position and score, how long it spends
//! choosing moves, and how long whole games take.

use clap::Parser;
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::rng::Rng;
use colosseum_engine::{Game, GameConfig};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const ALL_BOTS: &[&str] = &["planner", "auctioneer", "trader", "rollout", "showman", "oracle", "chameleon", "maestro", "house", "search", "live-input", "live-input-core", "tuned-1", "tuned-2", "int-event", "oracle-tuned", "greedy", "random"];

fn make_bot(name: &str, seed: u64) -> Option<Box<dyn Agent>> {
    Some(match name {
        "planner" => Box::new(colosseum_bot_planner::PlannerAgent::new(seed)),
        "auctioneer" => Box::new(colosseum_bot_auctioneer::AuctioneerAgent::new(seed)),
        "trader" => Box::new(colosseum_bot_trader::TraderAgent::new(seed)),
        "rollout" => Box::new(colosseum_bot_rollout::RolloutAgent::new(seed)),
        "showman" => Box::new(colosseum_bot_showman::ShowmanAgent::new(seed)),
        "oracle" => Box::new(colosseum_bot_oracle::OracleAgent::new(seed)),
        "search" => Box::new(colosseum_bot_search::SearchAgent::new(seed)),
        "live-input" => Box::new(colosseum_bot_live_input::LiveInputAgent::new(seed)),
        "live-input-core" => Box::new(colosseum_bot_live_input::CoreAgent::new(seed)),
        "tuned-1" => Box::new(colosseum_bot_tuned_1::Tuned1Agent::new(seed)),
        "tuned-2" => Box::new(colosseum_bot_tuned_2::Tuned2Agent::new(seed)),
        "int-event" => Box::new(colosseum_bot_int_event::IntEventAgent::new(seed)),
        // Variants: search-<base>-<rollout>-<budget>[-wide], e.g. search-house-house-200-wide.
        s if s.starts_with("search-") => {
            let p: Vec<&str> = s.split('-').collect();
            let cfg = colosseum_bot_search::Config {
                base: p.get(1)?.to_string(),
                rollout: p.get(2)?.to_string(),
                budget: p.get(3)?.parse().ok()?,
                wide: match p.get(4) {
                    None => false,
                    Some(&"wide") => true,
                    Some(_) => return None,
                },
                ..Default::default()
            };
            Box::new(colosseum_bot_search::SearchAgent::with_config(seed, cfg))
        }
        "oracle-tuned" => Box::new(colosseum_bot_oracle::OracleAgent::with_params(tuned_params().clone())),
        "chameleon" => Box::new(colosseum_bot_chameleon::ChameleonAgent::new(seed)),
        "maestro" => Box::new(colosseum_bot_maestro::MaestroAgent::new(seed)),
        "house" => Box::new(colosseum_bot_house::HouseAgent::new(seed)),
        "greedy" => Box::new(GreedyAgent::new(seed)),
        "random" => Box::new(RandomAgent::new(seed)),
        _ => return None,
    })
}

/// Params written by `train` (tuning/oracle/champion.json), or the defaults.
fn tuned_params() -> &'static colosseum_bot_oracle::Params {
    static P: std::sync::OnceLock<colosseum_bot_oracle::Params> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        std::fs::read_to_string("tuning/oracle/champion.json")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    })
}

#[derive(Parser)]
#[command(name = "tournament", about = "Pit Colosseum bots against each other and time the games")]
struct Cli {
    /// Bots in the pool (default: planner,auctioneer,trader,rollout,showman).
    #[arg(long, value_delimiter = ',', default_value = "planner,auctioneer,trader,rollout,showman")]
    bots: Vec<String>,
    /// Number of games to play.
    #[arg(long, default_value_t = 1000)]
    games: u64,
    /// Player counts to draw from.
    #[arg(long, value_delimiter = ',', default_value = "3,4,5")]
    sizes: Vec<usize>,
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Worker threads (default: available parallelism).
    #[arg(long)]
    threads: Option<usize>,
    /// Fill each seat independently from the pool, so a bot can hold several
    /// seats and tables can be larger than the pool. Adds a per-lineup report.
    #[arg(long)]
    repeat: bool,
}

#[derive(Default, Clone)]
struct BotStats {
    games: u64,
    wins: u64,
    /// Sum of (players - rank) / (players - 1): 1.0 for first, 0.0 for last.
    placement: f64,
    /// Sum of 1/players over seats: the wins expected at chance level.
    fair: f64,
    score: u64,
    decisions: u64,
    think: Duration,
    max_think: Duration,
    /// Win counts by table size.
    by_size: BTreeMap<usize, (u64, u64)>,
}

struct GameResult {
    size: usize,
    wall: Duration,
    actions: u64,
    seats: Vec<SeatResult>,
}

struct SeatResult {
    bot: String,
    rank: usize,
    score: u32,
    decisions: u64,
    think: Duration,
    max_think: Duration,
}

fn play_one(
    config: &Arc<GameConfig>,
    bots: &[String],
    sizes: &[usize],
    repeat: bool,
    game_seed: u64,
) -> Result<GameResult, String> {
    let mut rng = Rng::new(game_seed ^ 0xA5A5_5A5A_DEAD_BEEF);
    let seated: Vec<String> = if repeat {
        let size = sizes[rng.below(sizes.len())];
        (0..size).map(|_| bots[rng.below(bots.len())].clone()).collect()
    } else {
        let size = sizes[rng.below(sizes.len())].min(bots.len());
        let mut pool: Vec<&String> = bots.iter().collect();
        rng.shuffle(&mut pool);
        pool[..size].iter().map(|s| s.to_string()).collect()
    };
    let size = seated.len();

    let mut game = Game::new(config.clone(), size, game_seed).map_err(|e| e.to_string())?;
    let mut agents: Vec<Box<dyn Agent>> = seated
        .iter()
        .enumerate()
        .map(|(i, b)| make_bot(b, game_seed.wrapping_mul(31).wrapping_add(i as u64)).unwrap())
        .collect();
    let mut think = vec![Duration::ZERO; size];
    let mut max_think = vec![Duration::ZERO; size];
    let mut decisions = vec![0u64; size];
    let mut actions = 0u64;

    let start = Instant::now();
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let t = Instant::now();
        let action = agents[p.0].choose_action(&game, &legal);
        let dt = t.elapsed();
        think[p.0] += dt;
        max_think[p.0] = max_think[p.0].max(dt);
        decisions[p.0] += 1;
        game.apply(action.clone())
            .map_err(|e| format!("{} (seat {}) chose illegal {action:?}: {e:?} [seed {game_seed}]", seated[p.0], p.0))?;
        actions += 1;
        if actions > 200_000 {
            return Err(format!("game with seed {game_seed} exceeded 200000 actions ({seated:?})"));
        }
    }
    let wall = start.elapsed();

    let ranking = game.ranking();
    let seats = (0..size)
        .map(|i| SeatResult {
            bot: seated[i].clone(),
            rank: ranking.iter().position(|p| p.0 == i).unwrap(),
            score: game.players()[i].best_score,
            decisions: decisions[i],
            think: think[i],
            max_think: max_think[i],
        })
        .collect();
    Ok(GameResult { size, wall, actions, seats })
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    let cli = Cli::parse();
    for b in &cli.bots {
        if make_bot(b, 0).is_none() {
            eprintln!("unknown bot '{b}' (choose from {})", ALL_BOTS.join(", "));
            std::process::exit(1);
        }
    }
    // Some bots read tuning overrides from the environment; flag any that are set.
    let tuning: Vec<String> = std::env::vars()
        .map(|(k, _)| k)
        .filter(|k| k.starts_with("SH_") || k.starts_with("P_") || k == "TRADER_TUNE")
        .collect();
    if !tuning.is_empty() {
        eprintln!("warning: bot tuning overrides are set: {}", tuning.join(", "));
    }
    let config = Arc::new(GameConfig::embedded());
    let threads = cli
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));
    let next = AtomicU64::new(0);
    let total_start = Instant::now();

    let results: Vec<Result<GameResult, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let g = next.fetch_add(1, Ordering::Relaxed);
                        if g >= cli.games {
                            break;
                        }
                        let seed = cli.seed.wrapping_mul(1_000_003).wrapping_add(g);
                        out.push(play_one(&config, &cli.bots, &cli.sizes, cli.repeat, seed));
                        if (g + 1).is_multiple_of(500) {
                            eprintln!("  {} / {} games", g + 1, cli.games);
                        }
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("worker panicked")).collect()
    });
    let total = total_start.elapsed();

    let mut errors = Vec::new();
    let mut games = Vec::new();
    for r in results {
        match r {
            Ok(g) => games.push(g),
            Err(e) => errors.push(e),
        }
    }

    let mut stats: BTreeMap<String, BotStats> = BTreeMap::new();
    for g in &games {
        for s in &g.seats {
            let b = stats.entry(s.bot.clone()).or_default();
            b.games += 1;
            let won = s.rank == 0;
            b.wins += won as u64;
            b.placement += (g.size - 1 - s.rank) as f64 / (g.size - 1) as f64;
            b.fair += 1.0 / g.size as f64;
            b.score += s.score as u64;
            b.decisions += s.decisions;
            b.think += s.think;
            b.max_think = b.max_think.max(s.max_think);
            let e = b.by_size.entry(g.size).or_default();
            e.0 += won as u64;
            e.1 += 1;
        }
    }

    println!(
        "{} games ({} failed) on {threads} threads in {:.1}s, seed {}, sizes {:?}\n",
        games.len(),
        errors.len(),
        total.as_secs_f64(),
        cli.seed,
        cli.sizes
    );
    let mut rows: Vec<_> = stats.iter().collect();
    rows.sort_by(|a, b| (b.1.wins as f64 / b.1.games as f64).total_cmp(&(a.1.wins as f64 / a.1.games as f64)));
    let sizes: Vec<usize> = {
        let mut v = cli.sizes.clone();
        v.sort();
        v.dedup();
        v
    };
    print!("{:<11} {:>6} {:>7} {:>7} {:>7} {:>7}", "bot", "seats", "win %", "vs fair", "place", "score");
    for n in &sizes {
        print!(" {:>7}", format!("win@{n}p"));
    }
    println!(" {:>10} {:>10} {:>10}", "ms/move", "ms/game", "max move");
    for (name, b) in &rows {
        print!(
            "{:<11} {:>6} {:>6.1}% {:>6.2}x {:>7.3} {:>7.1}",
            name,
            b.games,
            100.0 * b.wins as f64 / b.games as f64,
            b.wins as f64 / b.fair,
            b.placement / b.games as f64,
            b.score as f64 / b.games as f64
        );
        for n in &sizes {
            match b.by_size.get(n) {
                Some((w, g)) if *g > 0 => print!(" {:>6.1}%", 100.0 * *w as f64 / *g as f64),
                _ => print!(" {:>7}", "-"),
            }
        }
        println!(
            " {:>10.4} {:>10.2} {:>9.1}ms",
            ms(b.think) / b.decisions.max(1) as f64,
            ms(b.think) / b.games as f64,
            ms(b.max_think)
        );
    }
    println!("(win % is per seat; vs fair = wins / chance-level wins, 1.00x = average)");
    println!("(place: 1.0 = always first, 0.0 = always last; chance level is 0.5)");

    if cli.repeat {
        // Lineup = how many seats each bot holds, ignoring seat order.
        let mut lineups: BTreeMap<(usize, Vec<usize>), (u64, Vec<u64>)> = BTreeMap::new();
        for g in &games {
            let counts: Vec<usize> = cli.bots.iter().map(|b| g.seats.iter().filter(|s| &s.bot == b).count()).collect();
            let e = lineups
                .entry((g.size, counts))
                .or_insert_with(|| (0, vec![0; cli.bots.len()]));
            e.0 += 1;
            let winner = g.seats.iter().find(|s| s.rank == 0).unwrap();
            e.1[cli.bots.iter().position(|b| *b == winner.bot).unwrap()] += 1;
        }
        println!("
Wins by lineup (share of games won by each bot type; '-' = not seated):");
        print!("{:<6} {:<16} {:>6}", "size", "lineup", "games");
        for b in &cli.bots {
            print!(" {:>11}", b);
        }
        println!();
        for ((size, counts), (n, wins)) in &lineups {
            let label: Vec<String> = counts
                .iter()
                .zip(&cli.bots)
                .filter(|(c, _)| **c > 0)
                .map(|(c, b)| format!("{c}{}", &b[..1].to_uppercase()))
                .collect();
            print!("{:<6} {:<16} {:>6}", format!("{size}p"), label.join("+"), n);
            for (i, c) in counts.iter().enumerate() {
                if *c == 0 {
                    print!(" {:>11}", "-");
                } else {
                    print!(" {:>10.1}%", 100.0 * wins[i] as f64 / *n as f64);
                }
            }
            println!();
        }
    }

    println!("\nGame duration by table size (wall clock, one thread):");
    println!(
        "{:<6} {:>6} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "size", "games", "mean ms", "p50 ms", "p95 ms", "max ms", "actions", "games/s"
    );
    let mut groups: BTreeMap<usize, Vec<&GameResult>> = BTreeMap::new();
    for g in &games {
        groups.entry(g.size).or_default().push(g);
    }
    groups.insert(0, games.iter().collect());
    for (size, gs) in &groups {
        let mut walls: Vec<f64> = gs.iter().map(|g| ms(g.wall)).collect();
        walls.sort_by(f64::total_cmp);
        let n = walls.len();
        if n == 0 {
            continue;
        }
        let mean = walls.iter().sum::<f64>() / n as f64;
        let actions = gs.iter().map(|g| g.actions).sum::<u64>() as f64 / n as f64;
        println!(
            "{:<6} {:>6} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9.0} {:>9.1}",
            if *size == 0 { "all".to_string() } else { format!("{size}p") },
            n,
            mean,
            walls[n / 2],
            walls[((n as f64 * 0.95) as usize).min(n - 1)],
            walls[n - 1],
            actions,
            1000.0 / mean
        );
    }
    println!(
        "Throughput: {:.1} games/s across all threads",
        games.len() as f64 / total.as_secs_f64()
    );

    if !errors.is_empty() {
        println!("\nFailures (first 10):");
        for e in errors.iter().take(10) {
            println!("  {e}");
        }
        std::process::exit(2);
    }
}
