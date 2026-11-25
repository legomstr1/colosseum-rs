//! Int-event plays only legal moves and finishes games at every table size,
//! and never asks for a Joker without giving more tokens than it asks for
//! or paying coins.

use colosseum_bot_int_event::IntEventAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Asset, Game, GameConfig, Phase};
use std::sync::Arc;

#[test]
fn plays_legal_games_with_considerate_offers() {
    let config = Arc::new(GameConfig::embedded());
    for n in 3..=5 {
        for seed in 0..6u64 {
            let mut game = Game::new(config.clone(), n, seed * 17 + n as u64).unwrap();
            let mut agents: Vec<Box<dyn Agent>> = (0..n)
                .map(|i| -> Box<dyn Agent> {
                    if i % 2 == 0 {
                        Box::new(IntEventAgent::new(seed + i as u64))
                    } else {
                        Box::new(colosseum_bot_house::HouseAgent::new(seed + i as u64))
                    }
                })
                .collect();
            while let Some(p) = game.current_player() {
                let legal = game.legal_actions();
                let a = agents[p.0].choose_action(&game, &legal);
                if p.0 % 2 == 0
                    && let Action::ProposeTrade { terms, .. } = &a
                    && matches!(game.phase(), Phase::Trading { .. })
                    && terms.receive.count(Asset::Joker) > 0
                {
                    assert!(terms.give.total() > terms.receive.total() || terms.give_coins > 0, "asked for a Joker cheaply: {terms:?}");
                }
                game.apply(a).unwrap();
            }
            assert!(game.is_over());
        }
    }
}
