use colosseum_bot_showman::ShowmanAgent;
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent, play_out};
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;

#[test]
fn plays_full_games_legally() {
    let config = Arc::new(GameConfig::embedded());
    for n in 3..=5 {
        for seed in 0..12u64 {
            let mut game = Game::new(config.clone(), n, seed).unwrap();
            let mut agents: Vec<Box<dyn Agent>> = (0..n)
                .map(|i| -> Box<dyn Agent> {
                    match (i + seed as usize) % 3 {
                        0 => Box::new(ShowmanAgent::new(seed)),
                        1 => Box::new(GreedyAgent::new(seed)),
                        _ => Box::new(RandomAgent::new(seed)),
                    }
                })
                .collect();
            play_out(&mut game, &mut agents);
            assert!(game.is_over());
        }
    }
}
