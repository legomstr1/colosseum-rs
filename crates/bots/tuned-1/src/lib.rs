//! Tuned-1: the live-input heuristic with considerate trade offers (see
//! `core`). It has no search layer.

mod core;
mod params;

pub use crate::core::Core;
pub use params::Params;

use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Game};

pub struct Tuned1Agent(pub Core);

impl Tuned1Agent {
    pub fn new(seed: u64) -> Self {
        Tuned1Agent(Core::new(seed, Params::default()))
    }

    pub fn with_params(seed: u64, p: Params) -> Self {
        Tuned1Agent(Core::new(seed, p))
    }
}

impl Agent for Tuned1Agent {
    fn name(&self) -> String {
        "tuned-1".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        self.0.choose(game, legal)
    }
}
