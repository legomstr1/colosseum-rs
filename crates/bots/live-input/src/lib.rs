//! Live-input: a Colosseum bot built from what live games against people
//! showed (see `core` for the heuristic and `search` for the Monte Carlo
//! layer on top of it).
//!
//! - `LiveInputAgent` ("live-input"): the tuned heuristic, with optional
//!   Monte Carlo search on top (off by default; see `search`).
//! - `CoreAgent` ("live-input-core"): the tuned heuristic alone, fast enough
//!   for sweeps and rollouts.

mod core;
mod params;
pub mod search;

pub use crate::core::Core;
pub use params::Params;
pub use search::{LiveInputAgent, SearchConfig};

use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Game};

/// The heuristic alone.
pub struct CoreAgent(pub Core);

impl CoreAgent {
    pub fn new(seed: u64) -> Self {
        CoreAgent(Core::new(seed, Params::default()))
    }

    pub fn with_params(seed: u64, p: Params) -> Self {
        CoreAgent(Core::new(seed, p))
    }
}

impl Agent for CoreAgent {
    fn name(&self) -> String {
        "live-input-core".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        self.0.choose(game, legal)
    }
}
