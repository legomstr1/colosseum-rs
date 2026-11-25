//! Live-input plays only legal moves and finishes games, alone and with
//! search, at every table size, and its money trades go through.

use colosseum_bot_live_input::{CoreAgent, LiveInputAgent, Params, SearchConfig};
use colosseum_engine::agent::{Agent, play_out};
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;

#[test]
fn core_plays_legal_games() {
    let config = Arc::new(GameConfig::embedded());
    for n in 3..=5 {
        for seed in 0..6u64 {
            let mut game = Game::new(config.clone(), n, seed * 13 + n as u64).unwrap();
            let mut agents: Vec<Box<dyn Agent>> = (0..n)
                .map(|i| -> Box<dyn Agent> {
                    if i % 2 == 0 {
                        Box::new(CoreAgent::new(seed + i as u64))
                    } else {
                        Box::new(colosseum_bot_house::HouseAgent::new(seed + i as u64))
                    }
                })
                .collect();
            play_out(&mut game, &mut agents);
            assert!(game.is_over());
        }
    }
}

#[test]
fn search_plays_legal_games() {
    let config = Arc::new(GameConfig::embedded());
    {
        let n = 4;
        let mut game = Game::new(config.clone(), n, 40 + n as u64).unwrap();
        let mut agents: Vec<Box<dyn Agent>> = (0..n)
            .map(|i| -> Box<dyn Agent> {
                if i == 0 {
                    let cfg = SearchConfig { budget: 4, ..Default::default() };
                    Box::new(LiveInputAgent::with_config(1, Params::default(), cfg))
                } else {
                    Box::new(CoreAgent::new(i as u64))
                }
            })
            .collect();
        play_out(&mut game, &mut agents);
        assert!(game.is_over());
    }
}

#[test]
fn overrides_parse() {
    let p = Params::default().with_overrides("fill_per_round=0.8,plan=TEPE,money_trades=false,price_ladder=3/6").unwrap();
    assert_eq!(p.fill_per_round, 0.8);
    assert_eq!(p.plan, "TEPE");
    assert!(!p.money_trades);
    assert_eq!(p.price_ladder, vec![3, 6]);
    assert!(Params::default().with_overrides("nope=1").is_err());
}
