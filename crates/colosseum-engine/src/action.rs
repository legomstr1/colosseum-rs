//! Every decision a player can make, as plain data.

use crate::types::{Assets, PlayerId, Side};
use serde::{Deserialize, Serialize};

/// The terms of a trade, from the proposer's point of view.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TradeTerms {
    pub give: Assets,
    pub give_coins: u32,
    pub receive: Assets,
    pub receive_coins: u32,
}

impl TradeTerms {
    pub fn is_empty(&self) -> bool {
        self.give.is_empty() && self.give_coins == 0 && self.receive.is_empty() && self.receive_coins == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    // Phase 1: investing
    BuyProgram { program: u8 },
    ExpandArena { side: Side },
    BuySeasonTicket,
    BuildLoge,
    /// Spend a SpecialEvent token for extra investments this phase.
    PlaySpecialEvent,
    /// Spend medals for an extra investment this phase.
    SpendMedalsForInvestment,
    EndInvesting,

    // Phase 2: acquiring
    StartAuction { market: usize, bid: u32 },
    /// Give up the right to start auctions this phase.
    PassInitiative,
    Bid { amount: u32 },
    PassBid,

    // Phase 3: trading. `ProposeTrade` and `CounterTrade` are open-ended, so
    // `Game::legal_actions` does not list them; check them with
    // `Game::is_legal`.
    ProposeTrade { to: PlayerId, terms: TradeTerms },
    AcceptTrade,
    RejectTrade,
    /// Answer an offer with new terms, from the counter-proposer's view.
    CounterTrade { terms: TradeTerms },
    EndTrading,

    // Phase 4: producing
    /// Move a noble clockwise by one die. `distance` is the die's value, or
    /// the chosen value for a choice face.
    MoveNoble { noble: usize, die: usize, distance: u8 },
    /// Move one noble by the sum of both dice (`distance`, with any choice
    /// face resolved).
    MoveNobleCombined { noble: usize, distance: u8 },
    /// Spend a medal to move a noble up to `max_noble_move` squares either way.
    MedalMoveNoble { noble: usize, delta: i8 },
    /// Produce an owned program, spending `medals` for extra spectators.
    ProduceEvent { program: u8, medals: u32 },
    SkipEvent,

    // Phase 5: closing ceremonies
    DiscardAsset { asset: crate::types::Asset },
    /// Name the asset type the podium winner must donate.
    RequestAsset { asset: crate::types::Asset },

    // Allowed whenever it is your decision
    ExchangeMedalForCoins,
    ExchangeSpecialEventForMedal,
}
