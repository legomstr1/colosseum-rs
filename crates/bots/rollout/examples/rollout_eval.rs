use colosseum_bot_rollout::RolloutAgent;
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;
use std::time::Duration;

#[cfg(windows)]
unsafe extern "system" {
    fn GetCurrentThread() -> isize;
    fn GetThreadTimes(h: isize, c: *mut u64, e: *mut u64, k: *mut u64, u: *mut u64) -> i32;
}

/// CPU time of this thread (immune to other processes competing for cores).
#[cfg(windows)]
fn cpu_now() -> Duration {
    let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
    unsafe { GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u) };
    Duration::from_nanos((k + u) * 100)
}

#[cfg(not(windows))]
fn cpu_now() -> Duration {
    Duration::ZERO
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let budget: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(colosseum_bot_rollout::DEFAULT_ROLLOUTS);
    let only: Option<String> = args.get(3).cloned();
    let config = Arc::new(GameConfig::embedded());
    for opp in ["greedy", "random"] {
        if only.as_deref().is_some_and(|o| o != opp) {
            continue;
        }
        for n in 3..=5usize {
            let (mut wins, mut played, mut share) = (0.0, 0, 0.0);
            let (mut total, mut max, mut calls) = (Duration::ZERO, Duration::ZERO, 0u64);
            for g in 0..games {
                let seat = g % n;
                let seed = 1000 + g as u64;
                let mut game = Game::new(config.clone(), n, seed).unwrap();
                let mut bot = RolloutAgent::with_budget(seed, budget);
                let mut others: Vec<Box<dyn Agent>> = (0..n)
                    .map(|i| -> Box<dyn Agent> {
                        if opp == "greedy" {
                            Box::new(GreedyAgent::new(seed + i as u64))
                        } else {
                            Box::new(RandomAgent::new(seed + i as u64))
                        }
                    })
                    .collect();
                while let Some(p) = game.current_player() {
                    let legal = game.legal_actions();
                    let a = if p.0 == seat {
                        let t = cpu_now();
                        let a = bot.choose_action(&game, &legal);
                        let d = cpu_now() - t;
                        total += d;
                        max = max.max(d);
                        calls += 1;
                        a
                    } else {
                        others[p.0].choose_action(&game, &legal)
                    };
                    game.apply(a).unwrap();
                }
                let r = game.ranking();
                let key = |p: usize| (game.player(r[p]).best_score, game.player(r[p]).coins);
                let top: Vec<_> = (0..n).filter(|&i| key(i) == key(0)).map(|i| r[i]).collect();
                if r[0].0 == seat {
                    wins += 1.0;
                }
                if top.iter().any(|p| p.0 == seat) {
                    share += 1.0 / top.len() as f64;
                }
                played += 1;
            }
            println!(
                "vs {opp:6} {n}p: win {:5.1}% (fair {:4.1}%) share {:5.1}%  avg {:.2} ms  max {:.1} ms  ({} calls/game)",
                100.0 * wins / played as f64,
                100.0 / n as f64,
                100.0 * share / played as f64,
                total.as_secs_f64() * 1000.0 / calls as f64,
                max.as_secs_f64() * 1000.0,
                calls / played as u64
            );
        }
    }
}
