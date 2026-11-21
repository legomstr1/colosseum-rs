//! Long-term program planner bot.
//!
//! The planner values any holding (tokens, programs, arena size, tickets,
//! coins) by projecting the score of its best final-round event
//! ([`model::Model::value`]) and picks, in every phase, the action that
//! raises that projection the most: which program to buy, when to grow the
//! arena or buy tickets, what a batch of tokens is worth in an auction, which
//! event to produce each round, and which token to throw away.

mod model;
mod phases;

pub use model::Params;

use colosseum_engine::agent::{Agent, GreedyAgent};
use colosseum_engine::{Action, Asset, Game, Phase};
use model::Model;

pub struct PlannerAgent {
    pub params: Params,
    fallback: GreedyAgent,
    /// Trade offers already made: `(round, partner, asset, price)`.
    pub(crate) tried: Vec<(u32, usize, Asset, u32)>,
}

impl PlannerAgent {
    pub fn new(seed: u64) -> Self {
        Self::with_params(seed, Params::default())
    }

    pub fn with_params(seed: u64, params: Params) -> Self {
        Self {
            params,
            fallback: GreedyAgent::new(seed ^ 0x5eed),
            tried: Vec::new(),
        }
    }
}

impl Agent for PlannerAgent {
    fn name(&self) -> String {
        "planner".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
        let params = self.params.clone();
        let m = Model {
            game,
            me,
            params: &params,
        };
        let chosen = match game.phase() {
            Phase::Investing { remaining, .. } => phases::invest(&m, legal, *remaining),
            Phase::Acquiring(acq) => phases::acquire(&m, legal, acq),
            Phase::Trading { offer: None, .. } => self.propose(&m, legal),
            Phase::Trading { offer: Some(o), .. } => Some(phases::respond(&m, legal, o)),
            Phase::Producing(pr) => phases::produce(&m, legal, pr),
            Phase::Closing(_) => phases::close(&m, legal),
            Phase::GameOver => None,
        };
        match chosen {
            Some(a) if legal.contains(&a) || game.is_legal(&a) => a,
            _ => self.fallback.choose_action(game, legal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colosseum_engine::agent::{RandomAgent, play_out};
    use colosseum_engine::GameConfig;
    use std::sync::Arc;

    #[test]
    fn plays_complete_games() {
        let config = Arc::new(GameConfig::embedded());
        for (n, seed) in [(3usize, 1u64), (4, 2), (5, 3), (3, 4)] {
            let mut game = Game::new(config.clone(), n, seed).unwrap();
            let mut agents: Vec<Box<dyn Agent>> = (0..n)
                .map(|i| -> Box<dyn Agent> {
                    if i % 2 == 0 {
                        Box::new(PlannerAgent::new(seed + i as u64))
                    } else {
                        Box::new(RandomAgent::new(seed + i as u64))
                    }
                })
                .collect();
            play_out(&mut game, &mut agents);
            assert!(game.is_over());
        }
    }
}
