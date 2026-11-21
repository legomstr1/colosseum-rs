//! Rules engine for the Colosseum board game (Days of Wonder, 2007).
//!
//! The game is plain data driven by a reducer:
//!
//! ```
//! use colosseum_engine::{Game, GameConfig};
//! use std::sync::Arc;
//!
//! let config = Arc::new(GameConfig::embedded());
//! let mut game = Game::new(config, 3, 42).unwrap();
//! while let Some(player) = game.current_player() {
//!     let legal = game.legal_actions();
//!     let events = game.apply(legal[legal.len() - 1].clone()).unwrap();
//!     # let _ = (player, events);
//! }
//! assert!(game.is_over());
//! ```
//!
//! - [`GameConfig`]: every rule constant and event card, loaded from YAML.
//! - [`Game`]: the full state, serializable, including the RNG.
//! - [`Action`]: every decision, as data. [`Game::legal_actions`] lists them.
//! - [`GameEvent`]: what happened, returned by [`Game::apply`].
//! - [`agent`]: computer players.

pub mod action;
pub mod agent;
pub mod config;
pub mod event;
mod game;
pub mod rng;
pub mod types;

pub use action::{Action, TradeTerms};
pub use config::{ConfigError, DieFace, EventDef, GameConfig, Rules};
pub use event::{GameEvent, PhaseKind, ScoreBreakdown};
pub use game::{
    Acquiring, Arena, Auction, Closing, EventPreview, Game, IllegalAction, Noble, Offer, Phase, PlayerState,
    ProducedEvent, Producing, Program, Supply,
};
pub use types::{Asset, Assets, NobleKind, PlayerId, Side};
