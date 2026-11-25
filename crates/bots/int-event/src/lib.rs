//! Int-event: season ticket, expansion, a tier-3 event, expansion, then a
//! tier-4 event in the final round, chasing podiums on the way (see `core`).

mod core;
mod params;

pub use crate::core::Core;
pub use params::Params;

use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Game};

pub struct IntEventAgent(pub Core);

impl IntEventAgent {
    pub fn new(seed: u64) -> Self {
        IntEventAgent(Core::new(seed, Params::default()))
    }

    pub fn with_params(seed: u64, p: Params) -> Self {
        IntEventAgent(Core::new(seed, p))
    }
}

impl Agent for IntEventAgent {
    fn name(&self) -> String {
        "int-event".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        self.0.choose(game, legal)
    }
}
