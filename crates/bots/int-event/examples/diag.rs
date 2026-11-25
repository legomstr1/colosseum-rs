//! Where int-event's points come from, next to the house bots it plays:
//! the average breakdown of each player's best event.
//!
//! cargo run --release -p colosseum-bot-int-event --example diag -- [games] [overrides]

use colosseum_bot_int_event::{IntEventAgent, Params};
use colosseum_engine::agent::Agent;
use colosseum_engine::event::GameEvent;
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;

#[derive(Default)]
struct Sum {
    n: f64,
    total: f64,
    base: f64,
    missing: f64,
    prev: f64,
    tickets: f64,
    stars: f64,
    nobles: f64,
    podiums: f64,
    medals: f64,
    base_score: f64,
    final_coins: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).map_or(300, |s| s.parse().unwrap());
    let over = args.get(2).cloned().unwrap_or_default();
    let params = Params::default().with_overrides(&over).unwrap();
    let config = Arc::new(GameConfig::embedded());
    let (mut mine, mut house) = (Sum::default(), Sum::default());
    for seed in 0..games {
        let n = 4;
        let mut game = Game::new(config.clone(), n, 9000 + seed).unwrap();
        let me = (seed % n as u64) as usize;
        let mut agents: Vec<Box<dyn Agent>> = (0..n)
            .map(|i| -> Box<dyn Agent> {
                if i == me {
                    Box::new(IntEventAgent::with_params(seed, params.clone()))
                } else {
                    Box::new(colosseum_bot_house::HouseAgent::new(seed * 7 + i as u64))
                }
            })
            .collect();
        let mut best: Vec<Option<colosseum_engine::event::ScoreBreakdown>> = vec![None; n];
        let mut best_base = vec![0u32; n];
        let mut final_coins = vec![0u32; n];
        while let Some(p) = game.current_player() {
            if game.round() == game.rules().num_rounds && matches!(game.phase(), colosseum_engine::Phase::Investing { .. }) {
                final_coins[p.0] = final_coins[p.0].max(game.player(p).coins);
            }
            let legal = game.legal_actions();
            let a = agents[p.0].choose_action(&game, &legal);
            for e in game.apply(a).unwrap() {
                if let GameEvent::EventProduced { player, program, breakdown, new_best: true, .. } = e {
                    best[player.0] = Some(breakdown);
                    best_base[player.0] = config.event(program).map_or(0, |d| d.base_score);
                }
            }
        }
        for i in 0..n {
            let s = if i == me { &mut mine } else { &mut house };
            let Some(b) = &best[i] else { continue };
            s.n += 1.0;
            s.total += b.total() as f64;
            s.base += b.base as f64;
            s.missing += b.missing_assets as f64;
            s.prev += b.previous_programs as f64;
            s.tickets += b.season_tickets as f64;
            s.stars += b.stars as f64;
            s.nobles += b.nobles as f64;
            s.podiums += b.podiums as f64;
            s.medals += b.medals as f64;
            s.base_score += best_base[i] as f64;
            s.final_coins += final_coins[i] as f64;
        }
    }
    println!("{:>10} {:>6} {:>6} {:>6} {:>7} {:>6} {:>7} {:>6} {:>6} {:>7} {:>6} {:>6}", "", "total", "event", "base", "missing", "prev", "tickets", "stars", "nobles", "podiums", "medals", "coins5");
    for (name, s) in [("int-event", &mine), ("house", &house)] {
        let k = s.n.max(1.0);
        println!(
            "{:>10} {:>6.1} {:>6.1} {:>6.1} {:>7.2} {:>6.1} {:>7.1} {:>6.1} {:>6.1} {:>7.1} {:>6.1} {:>6.1}",
            name,
            s.total / k,
            s.base_score / k,
            s.base / k,
            s.missing / k,
            s.prev / k,
            s.tickets / k,
            s.stars / k,
            s.nobles / k,
            s.podiums / k,
            s.medals / k,
            s.final_coins / k
        );
    }
}
