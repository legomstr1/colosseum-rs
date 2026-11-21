//! Full games at 3, 4 and 5 players: every action the oracle returns must be
//! legal (trade offers are checked with `is_legal`), and games must end.

use colosseum_bot_auctioneer::AuctioneerAgent;
use colosseum_bot_oracle::OracleAgent;
use colosseum_bot_planner::PlannerAgent;
use colosseum_bot_trader::TraderAgent;
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::{Action, Game, GameConfig};
use std::sync::Arc;

fn opponent(k: usize, seed: u64) -> Box<dyn Agent> {
    match k % 6 {
        0 => Box::new(PlannerAgent::new(seed)),
        1 => Box::new(AuctioneerAgent::new(seed)),
        2 => Box::new(TraderAgent::new(seed)),
        3 => Box::new(GreedyAgent::new(seed)),
        4 => Box::new(RandomAgent::new(seed)),
        _ => Box::new(OracleAgent::new(seed)),
    }
}

fn play(n: usize, seed: u64, oracle_seats: &[usize]) -> Game {
    let config = Arc::new(GameConfig::embedded());
    let mut game = Game::new(config, n, seed).unwrap();
    let mut agents: Vec<Box<dyn Agent>> = (0..n)
        .map(|i| {
            if oracle_seats.contains(&i) {
                Box::new(OracleAgent::new(seed + i as u64)) as Box<dyn Agent>
            } else {
                opponent(i + seed as usize, seed + i as u64)
            }
        })
        .collect();
    let mut steps = 0;
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let a = agents[p.0].choose_action(&game, &legal);
        if oracle_seats.contains(&p.0) {
            let ok = match &a {
                Action::ProposeTrade { .. } | Action::CounterTrade { .. } => game.is_legal(&a),
                _ => legal.contains(&a),
            };
            assert!(ok, "oracle returned an illegal action {a:?} (seed {seed})");
        }
        game.apply(a).expect("legal action");
        steps += 1;
        assert!(steps < 100_000, "game did not end (seed {seed})");
    }
    game
}

#[test]
fn full_games_against_every_bot() {
    for n in 3..=5 {
        for seed in 0..6u64 {
            assert!(play(n, seed * 13 + n as u64, &[0]).is_over());
            assert!(play(n, seed * 17 + 1, &[1, n - 1]).is_over());
        }
    }
}

#[test]
fn all_oracle_games_are_deterministic() {
    for n in 3..=5 {
        let a = play(n, 99 + n as u64, &(0..n).collect::<Vec<_>>());
        let b = play(n, 99 + n as u64, &(0..n).collect::<Vec<_>>());
        assert!(a.is_over());
        assert_eq!(a.history(), b.history());
    }
}
