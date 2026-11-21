//! Plays all-house games and reports how often each part of the strategy
//! fires: `cargo run --release -p colosseum-bot-house --example trace [games]`.
use colosseum_bot_house::HouseAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::{Asset, Game, GameConfig, GameEvent};
use std::sync::Arc;

fn main() {
    let config = Arc::new(GameConfig::embedded());
    let games: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let (mut win_sum, mut top, mut seats) = (0u64, 0u64, 0u64);
    let (mut joker_lots, mut joker_paid, mut se_tickets, mut medal_coins_early, mut medal_coins_final) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for seed in 0..games {
        let n = 4 + (seed % 2) as usize;
        let mut game = Game::new(config.clone(), n, seed).unwrap();
        let mut agents: Vec<Box<dyn Agent>> =
            (0..n).map(|i| Box::new(HouseAgent::new(seed * 7 + i as u64)) as Box<dyn Agent>).collect();
        while let Some(p) = game.current_player() {
            let legal = game.legal_actions();
            let a = agents[p.0].choose_action(&game, &legal);
            let round = game.round();
            let last = round >= game.rules().num_rounds;
            for e in game.apply(a).unwrap() {
                match e {
                    GameEvent::AuctionWon { price, assets, .. } if !last && assets.contains(&Asset::Joker) => {
                        joker_lots += 1;
                        joker_paid += price as u64;
                    }
                    GameEvent::ExtraInvestment { source, .. } if source.contains("SpecialEvent") => se_tickets += 1,
                    GameEvent::MedalExchangedForCoins { .. } if last => medal_coins_final += 1,
                    GameEvent::MedalExchangedForCoins { .. } => medal_coins_early += 1,
                    _ => {}
                }
            }
        }
        win_sum += game.players().iter().map(|p| p.best_score).max().unwrap() as u64;
        for p in game.players() {
            seats += 1;
            top += p.programs.iter().any(|q| q.id >= 25) as u64;
        }
    }
    let g = games as f64;
    println!(
        "games {games}: winner avg {:.1}, seats with a top-6 event {:.0}%",
        win_sum as f64 / g,
        100.0 * top as f64 / seats as f64
    );
    println!(
        "per game: Joker lots won in rounds 1-4 {:.2} (avg price {:.1}), SpecialEvent ticket buys {:.2}, medals sold early {:.2} / final round {:.2}",
        joker_lots as f64 / g,
        joker_paid as f64 / joker_lots.max(1) as f64,
        se_tickets as f64 / g,
        medal_coins_early as f64 / g,
        medal_coins_final as f64 / g
    );
}
