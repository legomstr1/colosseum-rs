//! Acquiring decisions taken from real games.

use colosseum_bot_live_input::CoreAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Game, GameConfig};
use std::sync::Arc;

/// Final round, game 86dd16686c94cb4d: P3 was outbid for the market it
/// wanted and kept the initiative with 9 coins and nobody left to bid
/// against it. Markets 0 and 3 each hold an Emperor token (a medal, worth
/// 3 points at the final event), so opening one at the minimum is free
/// points. The bot used to pass.
#[test]
fn opens_an_uncontested_emperor_market_in_the_final_round() {
    #[derive(serde::Deserialize)]
    struct Save {
        seats: Vec<String>,
        seed: u64,
        history: Vec<Action>,
    }
    let save: Save = serde_json::from_str(include_str!("fixtures/emperor_market.json")).unwrap();
    let config = Arc::new(GameConfig::embedded());
    let game = Game::replay(config, save.seats.len(), save.seed, &save.history[..277]).unwrap();
    assert_eq!(game.current_player().map(|p| p.0), Some(2));
    let legal = game.legal_actions();
    let action = CoreAgent::new(7).choose_action(&game, &legal);
    assert!(
        matches!(action, Action::StartAuction { market: 0 | 3, .. }),
        "expected to open an Emperor market, got {action:?}"
    );
}
