//! What happened as a result of an action: a structured log for UIs, replays
//! and agents.

use crate::action::TradeTerms;
use crate::config::DieFace;
use crate::types::{Asset, NobleKind, PlayerId};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseKind {
    Investing,
    Acquiring,
    Trading,
    Producing,
    Closing,
}

impl fmt::Display for PhaseKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PhaseKind::Investing => "Investing",
            PhaseKind::Acquiring => "Acquiring",
            PhaseKind::Trading => "Trading",
            PhaseKind::Producing => "Producing",
            PhaseKind::Closing => "Closing ceremonies",
        })
    }
}

/// How an event's spectator count was reached.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub missing_assets: u32,
    pub base: u32,
    pub previous_programs: u32,
    pub season_tickets: u32,
    pub stars: u32,
    pub nobles: u32,
    pub podiums: u32,
    pub medals: u32,
}

impl ScoreBreakdown {
    pub fn total(&self) -> u32 {
        self.base
            + self.previous_programs
            + self.season_tickets
            + self.stars
            + self.nobles
            + self.podiums
            + self.medals
    }
}

impl fmt::Display for ScoreBreakdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "base {}", self.base)?;
        if self.missing_assets > 0 {
            write!(f, " ({} missing)", self.missing_assets)?;
        }
        for (label, v) in [
            ("programs", self.previous_programs),
            ("tickets", self.season_tickets),
            ("stars", self.stars),
            ("nobles", self.nobles),
            ("podiums", self.podiums),
            ("medals", self.medals),
        ] {
            if v > 0 {
                write!(f, " + {label} {v}")?;
            }
        }
        write!(f, " = {}", self.total())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameEvent {
    RoundStarted { round: u32, start_player: PlayerId },
    PhaseStarted { phase: PhaseKind },
    ProgramBought { player: PlayerId, program: u8, name: String, cost: u32 },
    ArenaExpanded { player: PlayerId, square: u8 },
    SeasonTicketBought { player: PlayerId },
    LogeBuilt { player: PlayerId },
    ExtraInvestment { player: PlayerId, source: String },
    Passed { player: PlayerId },
    AuctionStarted { player: PlayerId, market: usize, bid: u32 },
    BidPlaced { player: PlayerId, amount: u32 },
    AuctionWon { player: PlayerId, market: usize, price: u32, assets: Vec<Asset> },
    MarketsRefilled,
    EmperorTokenConverted { player: PlayerId, medal_gained: bool },
    TradeProposed { from: PlayerId, to: PlayerId, terms: TradeTerms },
    TradeAccepted { from: PlayerId, to: PlayerId, terms: TradeTerms },
    TradeRejected { by: PlayerId },
    DiceRolled { player: PlayerId, dice: Vec<DieFace> },
    NobleMoved { player: PlayerId, kind: NobleKind, from: u8, to: u8, medal_earned: bool },
    EventProduced {
        player: PlayerId,
        program: u8,
        name: String,
        spectators: u32,
        breakdown: ScoreBreakdown,
        new_best: bool,
    },
    EventSkipped { player: PlayerId },
    MedalExchangedForCoins { player: PlayerId, coins: u32 },
    SpecialEventExchanged { player: PlayerId },
    StarAwardChanged { asset: Asset, from: Option<PlayerId>, to: Option<PlayerId> },
    PodiumAwarded { player: PlayerId },
    AssetDiscarded { player: PlayerId, asset: Asset },
    AssetDonated { from: PlayerId, to: PlayerId, asset: Asset },
    GameOver { ranking: Vec<PlayerId>, best_scores: Vec<u32> },
}

fn who(p: &Option<PlayerId>) -> String {
    p.map_or_else(|| "nobody".to_string(), |p| p.to_string())
}

impl fmt::Display for GameEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use GameEvent::*;
        match self {
            RoundStarted { round, start_player } => {
                write!(f, "=== Round {round} (start player {start_player}) ===")
            }
            PhaseStarted { phase } => write!(f, "--- {phase} ---"),
            ProgramBought { player, program, name, cost } => {
                write!(f, "{player} bought program #{program} {name} for {cost}")
            }
            ArenaExpanded { player, square } => {
                write!(f, "{player} expanded their arena to square {square}")
            }
            SeasonTicketBought { player } => write!(f, "{player} bought a season ticket"),
            LogeBuilt { player } => write!(f, "{player} built an Emperor's loge"),
            ExtraInvestment { player, source } => {
                write!(f, "{player} gained an extra investment ({source})")
            }
            Passed { player } => write!(f, "{player} passed"),
            AuctionStarted { player, market, bid } => {
                write!(f, "{player} opened market {} at {bid}", market + 1)
            }
            BidPlaced { player, amount } => write!(f, "{player} bid {amount}"),
            AuctionWon { player, market, price, assets } => {
                let names: Vec<String> = assets.iter().map(|a| a.to_string()).collect();
                write!(
                    f,
                    "{player} won market {} for {price}: {}",
                    market + 1,
                    names.join(", ")
                )
            }
            MarketsRefilled => write!(f, "Markets refilled"),
            EmperorTokenConverted { player, medal_gained } => {
                if *medal_gained {
                    write!(f, "{player} turned an Emperor token into a medal")
                } else {
                    write!(f, "{player}'s Emperor token was lost (no medals left)")
                }
            }
            TradeProposed { from, to, terms } => write!(
                f,
                "{from} offers {to}: {} + {} coins for {} + {} coins",
                terms.give, terms.give_coins, terms.receive, terms.receive_coins
            ),
            TradeAccepted { from, to, .. } => write!(f, "{to} accepted {from}'s offer"),
            TradeRejected { by } => write!(f, "{by} rejected the offer"),
            DiceRolled { player, dice } => {
                let dice: Vec<String> = dice.iter().map(ToString::to_string).collect();
                write!(f, "{player} rolled {}", dice.join(" and "))
            }
            NobleMoved { player, kind, from, to, medal_earned } => {
                write!(f, "{player} moved the {kind} from {from} to {to}")?;
                if *medal_earned {
                    write!(f, " and earned a medal")?;
                }
                Ok(())
            }
            EventProduced { player, program, name, spectators, breakdown, new_best } => {
                write!(
                    f,
                    "{player} produced #{program} {name}: {spectators} spectators ({breakdown})"
                )?;
                if *new_best {
                    write!(f, " - new best!")?;
                }
                Ok(())
            }
            EventSkipped { player } => write!(f, "{player} produced no event"),
            MedalExchangedForCoins { player, coins } => {
                write!(f, "{player} exchanged a medal for {coins} coins")
            }
            SpecialEventExchanged { player } => {
                write!(f, "{player} exchanged a SpecialEvent token for a medal")
            }
            StarAwardChanged { asset, from, to } => {
                write!(f, "{asset} star: {} -> {}", who(from), who(to))
            }
            PodiumAwarded { player } => write!(f, "{player} receives a podium"),
            AssetDiscarded { player, asset } => write!(f, "{player} discarded a {asset}"),
            AssetDonated { from, to, asset } => write!(f, "{from} donated a {asset} to {to}"),
            GameOver { ranking, best_scores } => {
                write!(f, "=== Game over ===")?;
                for (place, p) in ranking.iter().enumerate() {
                    write!(f, "\n  {}. {p}: {} spectators", place + 1, best_scores[p.0])?;
                }
                Ok(())
            }
        }
    }
}
