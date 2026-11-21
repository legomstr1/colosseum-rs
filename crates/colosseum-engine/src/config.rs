//! Game data loaded from YAML (`data/game_config.yml` and `data/events.yml`).
//! The engine reads every rule constant from here; nothing numeric is
//! hardcoded in the rules code.

use crate::types::{Asset, Assets, NobleKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

const EMBEDDED_RULES: &str = include_str!("../../../data/game_config.yml");
const EMBEDDED_EVENTS: &str = include_str!("../../../data/events.yml");

/// One event program card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDef {
    pub id: u8,
    pub name: String,
    pub cost: u32,
    pub base_score: u32,
    /// Index into `arena.squares_required_by_event_size`.
    pub size: u8,
    pub requirements: Vec<Asset>,
    /// `penalty_scores[i]` is the score with `i + 1` assets missing. Its length
    /// is the most assets that may be missing.
    pub penalty_scores: Vec<u32>,
    /// `requirements` as counts, computed on first use (bots ask often).
    #[serde(skip)]
    counts: CountsCache,
}

/// A cache, so it never makes two otherwise equal definitions differ.
#[derive(Clone, Debug, Default)]
struct CountsCache(std::sync::OnceLock<Assets>);

impl PartialEq for CountsCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for CountsCache {}

impl EventDef {
    pub fn requirement_counts(&self) -> Assets {
        self.counts.0.get_or_init(|| self.requirements.iter().copied().collect()).clone()
    }

    pub fn max_missing(&self) -> usize {
        self.penalty_scores.len()
    }

    /// Score before bonuses with `missing` assets missing, or `None` if too
    /// many are missing to produce it.
    pub fn score_with_missing(&self, missing: usize) -> Option<u32> {
        if missing == 0 {
            Some(self.base_score)
        } else {
            self.penalty_scores.get(missing - 1).copied()
        }
    }
}

#[derive(Debug, Deserialize)]
struct EventsFile {
    events: Vec<EventDef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardRules {
    pub size: u8,
}

/// One face of a die: a fixed number, or a set of numbers the roller picks
/// from when they use the die. In YAML, `4` or `[1, 2, 3]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DieFace {
    Fixed(u8),
    Choice(Vec<u8>),
}

impl DieFace {
    /// The distances this face allows.
    pub fn values(&self) -> &[u8] {
        match self {
            DieFace::Fixed(v) => std::slice::from_ref(v),
            DieFace::Choice(vs) => vs,
        }
    }
}

impl fmt::Display for DieFace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DieFace::Fixed(v) => write!(f, "{v}"),
            DieFace::Choice(vs) => {
                let vs: Vec<String> = vs.iter().map(u8::to_string).collect();
                write!(f, "choice of {}", vs.join("/"))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceRules {
    pub faces: Vec<DieFace>,
    pub base_count: usize,
    pub loge_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArenaRules {
    pub max_squares: u8,
    pub expansion_cost: u32,
    pub squares_required_by_event_size: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestingRules {
    pub investments_per_phase: u32,
    pub season_ticket_cost: u32,
    pub emperor_loge_cost: u32,
    pub extra_investment_medal_cost: u32,
    pub special_event_extra_investments: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquiringRules {
    pub markets: usize,
    pub batch_size: usize,
    pub minimum_opening_bid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradingRules {
    /// Offers the active player may make to each other player per turn.
    pub max_offers_per_partner: u32,
    /// Counter-offers that may pass between the active player and each
    /// other player per turn, counting both sides.
    pub max_counter_offers_per_partner: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramRules {
    pub starting_hand_pools: Vec<Vec<u8>>,
    /// Programs for sale. Any of them can be bought.
    pub for_sale: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoringRules {
    pub previous_program_bonus: u32,
    pub season_ticket_bonus: u32,
    pub star_award_bonus: u32,
    pub podium_bonus: u32,
    pub medal_bonus: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarRules {
    pub threshold: u32,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyRules {
    pub medals: u32,
    pub podiums: u32,
    pub season_tickets: u32,
    pub emperor_loges: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MedalRules {
    pub coin_value: u32,
    pub max_noble_move: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRules {
    pub total: BTreeMap<Asset, u32>,
    pub starting: BTreeMap<Asset, u32>,
    pub starting_per_player: BTreeMap<usize, u32>,
}

/// Everything in `game_config.yml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    pub num_rounds: u32,
    pub starting_coins: u32,
    pub board: BoardRules,
    pub starting_positions: BTreeMap<usize, Vec<Vec<u8>>>,
    pub noble_positions: Vec<(u8, NobleKind)>,
    pub noble_spectators: BTreeMap<NobleKind, u32>,
    pub dice: DiceRules,
    pub arena: ArenaRules,
    pub investing: InvestingRules,
    pub acquiring: AcquiringRules,
    pub trading: TradingRules,
    pub programs: ProgramRules,
    pub scoring: ScoringRules,
    pub stars: StarRules,
    pub supply: SupplyRules,
    pub medals: MedalRules,
    pub assets: AssetRules,
}

impl Rules {
    pub fn noble_value(&self, kind: NobleKind) -> u32 {
        self.noble_spectators.get(&kind).copied().unwrap_or(0)
    }

    pub fn is_resting_place(&self, square: u8) -> bool {
        self.noble_positions.iter().any(|(sq, _)| *sq == square)
    }
}

/// Complete, validated game data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameConfig {
    pub rules: Rules,
    pub events: Vec<EventDef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid game config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            return Err(ConfigError(format!($($arg)*)));
        }
    };
}

impl GameConfig {
    /// The data files compiled into the engine.
    pub fn embedded() -> Self {
        Self::from_yaml(EMBEDDED_RULES, EMBEDDED_EVENTS).expect("embedded game data is invalid")
    }

    pub fn from_yaml(rules_yaml: &str, events_yaml: &str) -> Result<Self, ConfigError> {
        let rules: Rules = serde_yml::from_str(rules_yaml)
            .map_err(|e| ConfigError(format!("game_config.yml: {e}")))?;
        let events: EventsFile = serde_yml::from_str(events_yaml)
            .map_err(|e| ConfigError(format!("events.yml: {e}")))?;
        let config = Self {
            rules,
            events: events.events,
        };
        config.validate()?;
        Ok(config)
    }

    /// Loads `game_config.yml` and `events.yml` from a directory.
    pub fn from_dir(dir: &Path) -> Result<Self, ConfigError> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name))
                .map_err(|e| ConfigError(format!("{}: {e}", dir.join(name).display())))
        };
        Self::from_yaml(&read("game_config.yml")?, &read("events.yml")?)
    }

    pub fn event(&self, id: u8) -> Option<&EventDef> {
        // Ids are usually 1..=n in file order; fall back to a scan otherwise.
        match self.events.get((id as usize).wrapping_sub(1)) {
            Some(e) if e.id == id => Some(e),
            _ => self.events.iter().find(|e| e.id == id),
        }
    }

    pub fn supported_player_counts(&self) -> Vec<usize> {
        self.rules
            .starting_positions
            .keys()
            .copied()
            .filter(|n| self.rules.assets.starting_per_player.contains_key(n))
            .collect()
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let r = &self.rules;
        ensure!(r.board.size > 0, "board.size must be positive");
        ensure!(!r.dice.faces.is_empty(), "dice.faces is empty");
        ensure!(
            r.dice.faces.iter().all(|f| !f.values().is_empty() && !f.values().contains(&0)),
            "every die face needs at least one positive value"
        );
        ensure!(r.acquiring.batch_size > 0, "acquiring.batch_size must be positive");
        ensure!(
            r.dice.base_count >= 1 && r.dice.loge_count >= 1,
            "dice counts must be at least 1"
        );

        let mut ids = BTreeSet::new();
        for e in &self.events {
            ensure!(ids.insert(e.id), "duplicate event id {}", e.id);
            ensure!(
                (e.size as usize) < r.arena.squares_required_by_event_size.len(),
                "event {} has size {} with no arena requirement",
                e.id,
                e.size
            );
            ensure!(
                e.penalty_scores.len() <= e.requirements.len(),
                "event {} has more penalty scores than requirements",
                e.id
            );
            ensure!(
                e.requirements.iter().all(|a| a.is_standard()),
                "event {} requires a special token",
                e.id
            );
        }
        for id in r
            .programs
            .starting_hand_pools
            .iter()
            .flatten()
            .chain(&r.programs.for_sale)
        {
            ensure!(ids.contains(id), "program {id} is not in events.yml");
        }
        for id in &r.programs.for_sale {
            ensure!(
                !r.programs.starting_hand_pools.iter().flatten().any(|s| s == id),
                "program {id} is both a starting program and for sale"
            );
        }

        for (asset, n) in &r.assets.starting {
            ensure!(
                r.assets.total.get(asset).copied().unwrap_or(0) >= *n,
                "more starting {asset} than total"
            );
        }
        let green: u32 = r.assets.starting.values().sum();
        let market_tokens = (r.acquiring.markets * r.acquiring.batch_size) as u32;

        ensure!(
            !self.supported_player_counts().is_empty(),
            "no player count has both starting_positions and starting_per_player"
        );
        for (&n, arenas) in &r.starting_positions {
            ensure!(arenas.len() == n, "starting_positions.{n} has {} arenas", arenas.len());
            let mut used = BTreeSet::new();
            for arena in arenas {
                ensure!(!arena.is_empty(), "empty arena in starting_positions.{n}");
                for (i, sq) in arena.iter().enumerate() {
                    ensure!(*sq < r.board.size, "square {sq} is off the board");
                    ensure!(used.insert(*sq), "square {sq} is in two arenas");
                    if i > 0 {
                        ensure!(
                            *sq == (arena[i - 1] + 1) % r.board.size,
                            "arena {arena:?} is not contiguous"
                        );
                    }
                }
            }
            for pool in &r.programs.starting_hand_pools {
                ensure!(pool.len() >= n, "a starting program pool is too small for {n} players");
            }
            if let Some(&per) = r.assets.starting_per_player.get(&n) {
                ensure!(
                    market_tokens + per * n as u32 <= green,
                    "not enough starting tokens for {n} players"
                );
            }
        }
        Ok(())
    }
}
