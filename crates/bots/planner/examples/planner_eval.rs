//! Plays the planner against GreedyAgent and RandomAgent.
//! Usage: eval [games_per_cell] [greedy|random|both] [key=value params...]

use colosseum_bot_planner::{Params, PlannerAgent};
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn set(p: &mut Params, k: &str, v: f64) {
    match k {
        "gain_rate" => p.gain_rate = v,
        "lambda" => p.lambda = v,
        "income" => p.income = v,
        "nobles" => p.nobles = v,
        "ticket_disc" => p.ticket_disc = v,
        "loge_value" => p.loge_value = v,
        "kappa" => p.kappa = v,
        "pay_frac" => p.pay_frac = v,
        "new_program" => p.new_program = v,
        "medal_value" => p.medal_value = v,
        "token_cost" => p.token_cost = v,
        "step_penalty" => p.step_penalty = v,
        "loge_per_round" => p.loge_per_round = v,
        "ticket_income" => p.ticket_income = v,
        _ => panic!("unknown param {k}"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut params = Params::default();
    let mut positional = Vec::new();
    for a in &args {
        if let Some((k, v)) = a.split_once('=') {
            set(&mut params, k, v.parse().expect("number"));
        } else {
            positional.push(a.clone());
        }
    }
    let games: usize = positional.first().and_then(|s| s.parse().ok()).unwrap_or(300);
    let only = positional.get(1).cloned().unwrap_or_else(|| "both".into());
    let config = Arc::new(GameConfig::embedded());
    for opp in ["greedy", "random"] {
        if only != "both" && only != opp {
            continue;
        }
        for n in 3..=5usize {
            let (mut wins, mut share, mut score, mut opp_score) = (0.0, 0.0, 0.0, 0.0);
            let (mut total, mut max, mut calls) = (Duration::ZERO, Duration::ZERO, 0u64);
            for g in 0..games {
                let seat = g % n;
                let seed = 1000 + g as u64;
                let mut game = Game::new(config.clone(), n, seed).unwrap();
                let mut bot = PlannerAgent::with_params(seed, params.clone());
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
                        let t = Instant::now();
                        let a = bot.choose_action(&game, &legal);
                        let d = t.elapsed();
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
                score += game.player(r[0]).best_score as f64 * 0.0
                    + game.players()[seat].best_score as f64;
                opp_score += (0..n)
                    .filter(|&i| i != seat)
                    .map(|i| game.players()[i].best_score as f64)
                    .sum::<f64>()
                    / (n - 1) as f64;
            }
            let gm = games as f64;
            println!(
                "vs {opp:6} {n}p: win {:5.1}% (fair {:4.1}%) share {:5.1}%  best {:5.1} vs {:5.1}  avg {:.3} ms max {:.1} ms ({} calls/game)",
                100.0 * wins / gm,
                100.0 / n as f64,
                100.0 * share / gm,
                score / gm,
                opp_score / gm,
                total.as_secs_f64() * 1000.0 / calls as f64,
                max.as_secs_f64() * 1000.0,
                calls / games as u64
            );
        }
    }
}
