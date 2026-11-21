//! Chameleon plays complete 3-, 4- and 5-player games against the existing
//! bots without ever choosing an illegal action, and does so deterministically.

use colosseum_bot_auctioneer::AuctioneerAgent;
use colosseum_bot_chameleon::ChameleonAgent;
use colosseum_bot_planner::PlannerAgent;
use colosseum_bot_trader::TraderAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Game, GameConfig};
use std::sync::Arc;

fn play(n: usize, seed: u64) -> Game {
    let config = Arc::new(GameConfig::embedded());
    let mut game = Game::new(config, n, seed).unwrap();
    let mut agents: Vec<Box<dyn Agent>> = (0..n)
        .map(|i| -> Box<dyn Agent> {
            let s = seed * 10 + i as u64;
            match (i + seed as usize) % 4 {
                0 => Box::new(ChameleonAgent::new(s)),
                1 => Box::new(PlannerAgent::new(s)),
                2 => Box::new(AuctioneerAgent::new(s)),
                _ => Box::new(TraderAgent::new(s)),
            }
        })
        .collect();
    let mut steps = 0;
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let a = agents[p.0].choose_action(&game, &legal);
        if agents[p.0].name() == "chameleon" {
            let ok = match &a {
                Action::ProposeTrade { .. } | Action::CounterTrade { .. } => game.is_legal(&a),
                other => legal.contains(other),
            };
            assert!(ok, "chameleon chose an illegal action {a:?} (n {n}, seed {seed})");
        }
        game.apply(a).expect("every bot chooses legal actions");
        steps += 1;
        assert!(steps < 50_000, "game did not terminate");
    }
    game
}

#[test]
fn plays_full_games_against_existing_bots() {
    for n in 3..=5 {
        for seed in 0..6u64 {
            let game = play(n, seed);
            assert!(game.is_over());
        }
    }
}

#[test]
fn is_deterministic() {
    for n in 3..=5 {
        let a = play(n, 42);
        let b = play(n, 42);
        assert_eq!(a.history(), b.history());
    }
}
