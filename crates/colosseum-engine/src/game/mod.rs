//! The game state and its reducer.
//!
//! `Game` is plain data: every piece of state, including the RNG, is a field
//! and the whole thing serializes. Play advances only through
//! [`Game::apply`], which checks an [`Action`] against the current [`Phase`],
//! updates the state, and returns the [`GameEvent`]s that happened. Each
//! phase's rules live in their own module as `*_actions` (what is legal) and
//! `apply_*` (what it does).

mod acquiring;
mod closing;
mod investing;
mod producing;
mod trading;
#[cfg(test)]
mod tests;

pub use acquiring::{Acquiring, Auction};
pub use closing::Closing;
pub use producing::{EventPreview, Producing};
pub use trading::Offer;

use crate::action::Action;
use crate::config::{ConfigError, GameConfig, Rules};
use crate::event::{GameEvent, PhaseKind};
use crate::rng::Rng;
use crate::types::{Asset, Assets, NobleKind, PlayerId, Side};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// A contiguous run of board squares, clockwise from `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arena {
    pub start: u8,
    pub len: u8,
}

impl Arena {
    pub fn squares(&self, board_size: u8) -> Vec<u8> {
        (0..self.len).map(|i| (self.start + i) % board_size).collect()
    }

    pub fn contains(&self, square: u8, board_size: u8) -> bool {
        let offset = (square as u32 + board_size as u32 - self.start as u32) % board_size as u32;
        offset < self.len as u32
    }

    /// The square an expansion on `side` would add.
    pub fn expansion_square(&self, side: Side, board_size: u8) -> u8 {
        match side {
            Side::Left => (self.start + board_size - 1) % board_size,
            Side::Right => (self.start + self.len) % board_size,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Program {
    pub id: u8,
    pub produced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducedEvent {
    pub round: u32,
    pub program: u8,
    pub spectators: u32,
    /// Tokens used, including Jokers. One is discarded at the closing.
    pub used: Assets,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: PlayerId,
    pub coins: u32,
    pub assets: Assets,
    pub arena: Arena,
    pub loge: bool,
    pub season_tickets: u32,
    pub medals: u32,
    pub podiums: u32,
    pub programs: Vec<Program>,
    /// Best single event so far: the score that wins the game.
    pub best_score: u32,
    pub last_event: Option<ProducedEvent>,
}

impl PlayerState {
    pub fn owns_program(&self, id: u8) -> bool {
        self.programs.iter().any(|p| p.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Noble {
    pub kind: NobleKind,
    pub position: u8,
}

/// Shared component supplies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Supply {
    pub medals: u32,
    pub podiums: u32,
    pub season_tickets: u32,
    pub emperor_loges: u32,
}

/// Where the game is, and the state that only matters inside that phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// `turn` counts players from the start player.
    Investing { turn: usize, remaining: u32 },
    Acquiring(Acquiring),
    /// `offers[q]` and `counters[q]` count this turn's offers to player `q`
    /// and the counter-offers exchanged with them.
    Trading {
        turn: usize,
        offers: Vec<u32>,
        counters: Vec<u32>,
        offer: Option<Offer>,
    },
    Producing(Producing),
    Closing(Closing),
    GameOver,
}

impl Phase {
    pub fn kind(&self) -> Option<PhaseKind> {
        Some(match self {
            Phase::Investing { .. } => PhaseKind::Investing,
            Phase::Acquiring(_) => PhaseKind::Acquiring,
            Phase::Trading { .. } => PhaseKind::Trading,
            Phase::Producing(_) => PhaseKind::Producing,
            Phase::Closing(_) => PhaseKind::Closing,
            Phase::GameOver => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IllegalAction {
    pub action: Box<Action>,
    pub reason: String,
}

impl fmt::Display for IllegalAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "illegal action {:?}: {}", self.action, self.reason)
    }
}

impl std::error::Error for IllegalAction {}

/// The full game state. Every field is private: the only way to change a
/// game is [`Game::apply`], which checks legality and records the action in
/// [`Game::history`]. The getters below hand out read-only views.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    config: Arc<GameConfig>,
    seed: u64,
    players: Vec<PlayerState>,
    round: u32,
    start_player: PlayerId,
    phase: Phase,
    nobles: Vec<Noble>,
    markets: Vec<Vec<Asset>>,
    /// Leftover green-backed tokens, used before the bag to refill markets.
    green_pool: Vec<Asset>,
    bag: Vec<Asset>,
    /// Program ids still for sale.
    programs_for_sale: Vec<u8>,
    supply: Supply,
    stars: BTreeMap<Asset, Option<PlayerId>>,
    /// Tokens removed from the game at closing ceremonies.
    discarded: Assets,
    /// Every action applied since setup. Replaying it from `seed` rebuilds
    /// this state exactly.
    history: Vec<Action>,
    /// False in search copies ([`Game::determinized`]), which never replay
    /// and are cloned often: their history keeps only the last action (bots
    /// read it to see how their trade offer was answered).
    #[serde(default = "yes")]
    record_history: bool,
    rng: Rng,
}

fn yes() -> bool {
    true
}

/// What `advance` should do next: the automatic steps between decisions.
enum Step {
    Wait,
    StartAcquiring,
    SetInitiator(PlayerId),
    EndAcquiring,
    StartProducing,
    Roll(usize),
    EndProducing,
    ClearDonation,
    EndRound,
}

impl Game {
    pub fn new(config: Arc<GameConfig>, num_players: usize, seed: u64) -> Result<Self, ConfigError> {
        let r = &config.rules;
        let positions = r.starting_positions.get(&num_players).ok_or_else(|| {
            ConfigError(format!(
                "{num_players} players not supported (supported: {:?})",
                config.supported_player_counts()
            ))
        })?;
        let per_player = *r
            .assets
            .starting_per_player
            .get(&num_players)
            .ok_or_else(|| ConfigError(format!("no starting_per_player for {num_players}")))?;

        let mut rng = Rng::new(seed);
        let expand = |m: &BTreeMap<Asset, u32>| -> Vec<Asset> {
            m.iter()
                .flat_map(|(a, n)| std::iter::repeat_n(*a, *n as usize))
                .collect()
        };
        let mut green_pool = expand(&r.assets.starting);
        rng.shuffle(&mut green_pool);
        let orange: BTreeMap<Asset, u32> = r
            .assets
            .total
            .iter()
            .map(|(a, n)| (*a, n - r.assets.starting.get(a).copied().unwrap_or(0)))
            .collect();
        let mut bag = expand(&orange);
        rng.shuffle(&mut bag);

        let mut hands: Vec<Vec<Program>> = vec![Vec::new(); num_players];
        for pool in &r.programs.starting_hand_pools {
            let mut pool = pool.clone();
            rng.shuffle(&mut pool);
            for (hand, id) in hands.iter_mut().zip(pool) {
                hand.push(Program { id, produced: false });
            }
        }

        let players = positions
            .iter()
            .zip(hands)
            .enumerate()
            .map(|(i, (squares, programs))| PlayerState {
                id: PlayerId(i),
                coins: r.starting_coins,
                assets: Assets::new(),
                arena: Arena {
                    start: squares[0],
                    len: squares.len() as u8,
                },
                loge: false,
                season_tickets: 0,
                medals: 0,
                podiums: 0,
                programs,
                best_score: 0,
                last_event: None,
            })
            .collect();

        let mut game = Game {
            seed,
            players,
            round: 1,
            start_player: PlayerId(0),
            phase: Phase::GameOver,
            nobles: r
                .noble_positions
                .iter()
                .map(|&(position, kind)| Noble { kind, position })
                .collect(),
            markets: vec![Vec::new(); r.acquiring.markets],
            green_pool,
            bag,
            programs_for_sale: r.programs.for_sale.clone(),
            supply: Supply {
                medals: r.supply.medals,
                podiums: r.supply.podiums,
                season_tickets: r.supply.season_tickets,
                emperor_loges: r.supply.emperor_loges,
            },
            stars: r.stars.assets.iter().map(|a| (*a, None)).collect(),
            discarded: Assets::new(),
            history: Vec::new(),
            record_history: true,
            rng,
            config: config.clone(),
        };

        let mut setup_log = Vec::new();
        game.refill_markets(&mut setup_log);
        for i in 0..num_players {
            let dealt: Vec<Asset> = (0..per_player).filter_map(|_| game.draw_token()).collect();
            game.add_assets(PlayerId(i), dealt, &mut setup_log);
        }
        // Stars are awarded once everyone is dealt, so players tied for the
        // most of a type at the start all go without.
        game.update_stars(&mut setup_log);
        game.start_player = game.first_player_after_emperor();
        game.start_round(&mut setup_log);
        Ok(game)
    }

    /// Rebuilds a game from its seed and action history.
    pub fn replay(
        config: Arc<GameConfig>,
        num_players: usize,
        seed: u64,
        actions: &[Action],
    ) -> Result<Self, String> {
        let mut game = Game::new(config, num_players, seed).map_err(|e| e.to_string())?;
        for a in actions {
            game.apply(a.clone()).map_err(|e| e.to_string())?;
        }
        Ok(game)
    }

    // ---- Queries -------------------------------------------------------

    pub fn config(&self) -> &GameConfig {
        &self.config
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn players(&self) -> &[PlayerState] {
        &self.players
    }

    /// The current round, starting at 1.
    pub fn round(&self) -> u32 {
        self.round
    }

    pub fn start_player(&self) -> PlayerId {
        self.start_player
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn nobles(&self) -> &[Noble] {
        &self.nobles
    }

    /// The token batches on offer in the acquiring phase.
    pub fn markets(&self) -> &[Vec<Asset>] {
        &self.markets
    }

    /// Leftover green-backed tokens, used before the bag to refill markets.
    pub fn green_pool(&self) -> &[Asset] {
        &self.green_pool
    }

    pub fn bag(&self) -> &[Asset] {
        &self.bag
    }

    /// A copy for search: what no player can know (the order of the bag and
    /// the green pool, and future dice) is redrawn from `seed`. To keep copies
    /// cheap, the history is dropped and from then on keeps only the last
    /// action. The result is a plausible world, not one reachable from
    /// `self.seed`, so it can't be replayed.
    pub fn determinized(&self, seed: u64) -> Game {
        let mut g = self.clone_without_history();
        g.rng = Rng::new(seed);
        g.rng.shuffle(&mut g.bag);
        g.rng.shuffle(&mut g.green_pool);
        g
    }

    fn clone_without_history(&self) -> Game {
        Game {
            config: self.config.clone(),
            seed: self.seed,
            players: self.players.clone(),
            round: self.round,
            start_player: self.start_player,
            phase: self.phase.clone(),
            nobles: self.nobles.clone(),
            markets: self.markets.clone(),
            green_pool: self.green_pool.clone(),
            bag: self.bag.clone(),
            programs_for_sale: self.programs_for_sale.clone(),
            supply: self.supply.clone(),
            stars: self.stars.clone(),
            discarded: self.discarded.clone(),
            history: Vec::new(),
            record_history: false,
            rng: self.rng.clone(),
        }
    }

    /// Program ids still for sale.
    pub fn programs_for_sale(&self) -> &[u8] {
        &self.programs_for_sale
    }

    pub fn supply(&self) -> &Supply {
        &self.supply
    }

    /// Each star award and who holds it.
    pub fn stars(&self) -> &BTreeMap<Asset, Option<PlayerId>> {
        &self.stars
    }

    /// Tokens removed from the game at closing ceremonies.
    pub fn discarded(&self) -> &Assets {
        &self.discarded
    }

    /// Every action applied since setup. Replaying it from the seed rebuilds
    /// this state exactly.
    pub fn history(&self) -> &[Action] {
        &self.history
    }

    pub fn rules(&self) -> &Rules {
        &self.config.rules
    }

    pub fn num_players(&self) -> usize {
        self.players.len()
    }

    pub fn player(&self, id: PlayerId) -> &PlayerState {
        &self.players[id.0]
    }

    pub fn is_over(&self) -> bool {
        self.phase == Phase::GameOver
    }

    /// The player in seat `turn` counting clockwise from the start player.
    pub fn seat(&self, turn: usize) -> PlayerId {
        PlayerId((self.start_player.0 + turn) % self.num_players())
    }

    /// The player who must make the next decision, or `None` once the game
    /// is over.
    pub fn current_player(&self) -> Option<PlayerId> {
        match &self.phase {
            Phase::Investing { turn, .. } => Some(self.seat(*turn)),
            Phase::Acquiring(a) => Some(a.auction.as_ref().map_or(a.cursor, |au| au.to_act)),
            Phase::Trading { turn, offer, .. } => {
                Some(offer.as_ref().map_or_else(|| self.seat(*turn), |o| o.to))
            }
            Phase::Producing(p) => Some(self.seat(p.turn)),
            Phase::Closing(c) => c
                .discard_queue
                .first()
                .copied()
                .or(c.donation.map(|(receiver, _)| receiver)),
            Phase::GameOver => None,
        }
    }

    /// Every legal action for the current player, except open-ended trade
    /// offers (`ProposeTrade`, `CounterTrade`), which [`Game::is_legal`]
    /// checks instead.
    pub fn legal_actions(&self) -> Vec<Action> {
        let Some(p) = self.current_player() else {
            return Vec::new();
        };
        let mut actions = match &self.phase {
            Phase::Investing { remaining, .. } => self.investing_actions(p, *remaining),
            Phase::Acquiring(a) => self.acquiring_actions(p, a),
            Phase::Trading { offer, .. } => self.trading_actions(offer.as_ref()),
            Phase::Producing(pr) => self.producing_actions(p, pr),
            Phase::Closing(c) => self.closing_actions(p, c),
            Phase::GameOver => Vec::new(),
        };
        let me = self.player(p);
        if me.medals > 0 {
            actions.push(Action::ExchangeMedalForCoins);
        }
        if me.assets.count(Asset::SpecialEvent) > 0 && self.supply.medals > 0 {
            actions.push(Action::ExchangeSpecialEventForMedal);
        }
        actions
    }

    /// Whether `action` may be applied now, with the reason if not.
    pub fn check(&self, action: &Action) -> Result<(), String> {
        let p = self.current_player().ok_or("the game is over")?;
        match action {
            Action::ProposeTrade { to, terms } => self.check_proposal(p, *to, terms),
            Action::CounterTrade { terms } => self.check_counter(p, terms),
            a if self.legal_actions().contains(a) => Ok(()),
            _ => Err("not a legal action right now".into()),
        }
    }

    pub fn is_legal(&self, action: &Action) -> bool {
        self.check(action).is_ok()
    }

    /// Players ordered by final standing: best single event, then coins,
    /// then star awards held.
    pub fn ranking(&self) -> Vec<PlayerId> {
        let mut ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        ids.sort_by_key(|&id| {
            let p = self.player(id);
            std::cmp::Reverse((p.best_score, p.coins, self.stars_held(id)))
        });
        ids
    }

    pub fn stars_held(&self, id: PlayerId) -> usize {
        self.stars.values().filter(|h| **h == Some(id)).count()
    }

    pub fn arena_owner(&self, square: u8) -> Option<PlayerId> {
        let size = self.rules().board.size;
        self.players
            .iter()
            .find(|p| p.arena.contains(square, size))
            .map(|p| p.id)
    }

    /// Indices of the nobles standing in a player's arena.
    pub fn nobles_in_arena(&self, id: PlayerId) -> Vec<usize> {
        let size = self.rules().board.size;
        let arena = self.player(id).arena;
        (0..self.nobles.len())
            .filter(|&i| arena.contains(self.nobles[i].position, size))
            .collect()
    }

    // ---- The reducer ---------------------------------------------------

    /// Applies the current player's action and runs any automatic steps
    /// that follow, returning everything that happened.
    pub fn apply(&mut self, action: Action) -> Result<Vec<GameEvent>, IllegalAction> {
        if let Err(reason) = self.check(&action) {
            return Err(IllegalAction {
                action: Box::new(action),
                reason,
            });
        }
        let p = self.current_player().expect("checked above");
        if !self.record_history {
            self.history.clear();
        }
        self.history.push(action.clone());
        let mut ev = Vec::new();
        match action {
            Action::ExchangeMedalForCoins => {
                let coins = self.rules().medals.coin_value;
                let me = &mut self.players[p.0];
                me.medals -= 1;
                me.coins += coins;
                self.supply.medals += 1;
                ev.push(GameEvent::MedalExchangedForCoins { player: p, coins });
            }
            Action::ExchangeSpecialEventForMedal => {
                self.players[p.0].assets.remove(Asset::SpecialEvent, 1);
                self.players[p.0].medals += 1;
                self.supply.medals -= 1;
                ev.push(GameEvent::SpecialEventExchanged { player: p });
            }
            action => match self.phase {
                Phase::Investing { .. } => self.apply_investing(p, action, &mut ev),
                Phase::Acquiring(_) => self.apply_acquiring(p, action, &mut ev),
                Phase::Trading { .. } => self.apply_trading(p, action, &mut ev),
                Phase::Producing(_) => self.apply_producing(p, action, &mut ev),
                Phase::Closing(_) => self.apply_closing(p, action, &mut ev),
                Phase::GameOver => unreachable!("check rejects actions after the game ends"),
            },
        }
        self.advance(&mut ev);
        Ok(ev)
    }

    /// Runs automatic steps until a player has a decision to make.
    fn advance(&mut self, ev: &mut Vec<GameEvent>) {
        loop {
            match self.next_step() {
                Step::Wait => return,
                Step::StartAcquiring => self.start_acquiring(ev),
                Step::SetInitiator(p) => {
                    if let Phase::Acquiring(a) = &mut self.phase {
                        a.cursor = p;
                    }
                    return;
                }
                Step::EndAcquiring => {
                    self.refill_markets(ev);
                    self.start_phase(
                        Phase::Trading {
                            turn: 0,
                            offers: vec![0; self.players.len()],
                            counters: vec![0; self.players.len()],
                            offer: None,
                        },
                        ev,
                    );
                }
                Step::StartProducing => self.start_phase(Phase::Producing(Producing::default()), ev),
                Step::Roll(turn) => self.roll_dice(turn, ev),
                Step::EndProducing => {
                    if self.round >= self.rules().num_rounds {
                        self.finish(ev);
                    } else {
                        self.start_closing(ev);
                    }
                }
                Step::ClearDonation => {
                    if let Phase::Closing(c) = &mut self.phase {
                        c.donation = None;
                    }
                }
                Step::EndRound => {
                    self.round += 1;
                    self.start_player = self.seat(1);
                    self.refill_markets(ev);
                    self.start_round(ev);
                }
            }
        }
    }

    fn next_step(&self) -> Step {
        let n = self.num_players();
        match &self.phase {
            Phase::Investing { turn, .. } if *turn >= n => Step::StartAcquiring,
            Phase::Acquiring(a) if a.auction.is_none() => (0..n)
                .map(|k| PlayerId((a.cursor.0 + k) % n))
                .find(|p| !a.done[p.0])
                .map_or(Step::EndAcquiring, |p| {
                    if p == a.cursor {
                        Step::Wait
                    } else {
                        Step::SetInitiator(p)
                    }
                }),
            Phase::Trading { turn, .. } if *turn >= n => Step::StartProducing,
            Phase::Producing(p) if p.turn >= n => Step::EndProducing,
            Phase::Producing(p) if !p.rolled => Step::Roll(p.turn),
            Phase::Closing(c) if c.discard_queue.is_empty() => match c.donation {
                Some((receiver, donor))
                    if receiver != donor && self.player(donor).assets.iter().any(|(a, _)| a.is_standard()) =>
                {
                    Step::Wait
                }
                Some(_) => Step::ClearDonation,
                None => Step::EndRound,
            },
            _ => Step::Wait,
        }
    }

    // ---- Transitions and shared helpers --------------------------------

    fn start_round(&mut self, ev: &mut Vec<GameEvent>) {
        ev.push(GameEvent::RoundStarted {
            round: self.round,
            start_player: self.start_player,
        });
        let remaining = self.rules().investing.investments_per_phase;
        self.start_phase(Phase::Investing { turn: 0, remaining }, ev);
    }

    fn start_phase(&mut self, phase: Phase, ev: &mut Vec<GameEvent>) {
        if let Some(kind) = phase.kind() {
            ev.push(GameEvent::PhaseStarted { phase: kind });
        }
        self.phase = phase;
    }

    fn finish(&mut self, ev: &mut Vec<GameEvent>) {
        self.phase = Phase::GameOver;
        ev.push(GameEvent::GameOver {
            ranking: self.ranking(),
            best_scores: self.players.iter().map(|p| p.best_score).collect(),
        });
    }

    /// The first arena clockwise from the Emperor's square.
    fn first_player_after_emperor(&self) -> PlayerId {
        let size = self.rules().board.size;
        let emperor = self
            .nobles
            .iter()
            .find(|n| n.kind == NobleKind::Emperor)
            .map_or(0, |n| n.position);
        (1..=size)
            .find_map(|d| self.arena_owner((emperor + d) % size))
            .unwrap_or(PlayerId(0))
    }

    fn draw_token(&mut self) -> Option<Asset> {
        self.green_pool.pop().or_else(|| self.bag.pop())
    }

    /// Fills every empty market from the green pool, then the bag.
    fn refill_markets(&mut self, ev: &mut Vec<GameEvent>) {
        let batch = self.rules().acquiring.batch_size;
        let mut refilled = false;
        for m in 0..self.markets.len() {
            if self.markets[m].is_empty() {
                let tokens: Vec<Asset> = (0..batch).filter_map(|_| self.draw_token()).collect();
                refilled |= !tokens.is_empty();
                self.markets[m] = tokens;
            }
        }
        if refilled {
            ev.push(GameEvent::MarketsRefilled);
        }
    }

    /// Gives tokens to a player. Emperor tokens become medals on the spot.
    fn gain_assets(&mut self, p: PlayerId, tokens: impl IntoIterator<Item = Asset>, ev: &mut Vec<GameEvent>) {
        self.add_assets(p, tokens, ev);
        self.update_stars(ev);
    }

    /// `gain_assets` without re-awarding stars, for dealing several players
    /// at once.
    fn add_assets(&mut self, p: PlayerId, tokens: impl IntoIterator<Item = Asset>, ev: &mut Vec<GameEvent>) {
        for token in tokens {
            if token == Asset::Emperor {
                let medal_gained = self.supply.medals > 0;
                if medal_gained {
                    self.supply.medals -= 1;
                    self.players[p.0].medals += 1;
                }
                ev.push(GameEvent::EmperorTokenConverted { player: p, medal_gained });
            } else {
                self.players[p.0].assets.add(token, 1);
            }
        }
    }

    /// Re-awards star performers after any change in asset holdings.
    fn update_stars(&mut self, ev: &mut Vec<GameEvent>) {
        let config = self.config.clone();
        let threshold = config.rules.stars.threshold;
        for &asset in &config.rules.stars.assets {
            let counts: Vec<u32> = self.players.iter().map(|p| p.assets.count(asset)).collect();
            let max = counts.iter().copied().max().unwrap_or(0);
            let leaders: Vec<usize> = (0..counts.len()).filter(|&i| counts[i] == max).collect();
            let leader = (max >= threshold && leaders.len() == 1).then(|| PlayerId(leaders[0]));
            let holder = self.stars.get(&asset).copied().flatten();
            let new = match holder {
                Some(h) if counts[h.0] >= threshold => match leader {
                    Some(l) if counts[l.0] > counts[h.0] => Some(l),
                    _ => Some(h),
                },
                _ => leader,
            };
            if new != holder {
                self.stars.insert(asset, new);
                ev.push(GameEvent::StarAwardChanged {
                    asset,
                    from: holder,
                    to: new,
                });
            }
        }
    }

    /// One-line, human-readable description of an action in this game.
    pub fn describe_action(&self, action: &Action) -> String {
        let program = |id: u8| {
            self.config
                .event(id)
                .map_or_else(|| format!("#{id}"), |e| format!("#{id} {}", e.name))
        };
        let noble = |i: usize| {
            self.nobles
                .get(i)
                .map_or_else(|| format!("noble {i}"), |n| format!("{} (at {})", n.kind, n.position))
        };
        match action {
            Action::BuyProgram { program: id } => {
                let cost = self.config.event(*id).map_or(0, |e| e.cost);
                format!("Buy program {} for {cost}", program(*id))
            }
            Action::ExpandArena { side } => {
                let me = self.current_player().map(|p| self.player(p).arena);
                let sq = me.map(|a| a.expansion_square(*side, self.rules().board.size));
                format!(
                    "Expand arena {side} to square {} for {}",
                    sq.unwrap_or(0),
                    self.rules().arena.expansion_cost
                )
            }
            Action::BuySeasonTicket => {
                format!("Buy a season ticket for {}", self.rules().investing.season_ticket_cost)
            }
            Action::BuildLoge => {
                format!("Build an Emperor's loge for {}", self.rules().investing.emperor_loge_cost)
            }
            Action::PlaySpecialEvent => "Play a SpecialEvent token for extra investments".into(),
            Action::SpendMedalsForInvestment => format!(
                "Spend {} medals for an extra investment",
                self.rules().investing.extra_investment_medal_cost
            ),
            Action::EndInvesting => "End investing".into(),
            Action::StartAuction { market, bid } => {
                let tokens: Assets = self.markets.get(*market).into_iter().flatten().copied().collect();
                format!("Open market {} ({tokens}) at {bid}", market + 1)
            }
            Action::PassInitiative => "Pass (start no more auctions)".into(),
            Action::Bid { amount } => format!("Bid {amount}"),
            Action::PassBid => "Pass on this auction".into(),
            Action::ProposeTrade { to, terms } => format!(
                "Offer {to}: {} + {} coins for {} + {} coins",
                terms.give, terms.give_coins, terms.receive, terms.receive_coins
            ),
            Action::AcceptTrade => "Accept the offer".into(),
            Action::RejectTrade => "Reject the offer".into(),
            Action::CounterTrade { terms } => format!(
                "Counter: {} + {} coins for {} + {} coins",
                terms.give, terms.give_coins, terms.receive, terms.receive_coins
            ),
            Action::EndTrading => "End trading".into(),
            Action::MoveNoble { noble: i, die, distance } => {
                format!("Move {} by {distance} (die {})", noble(*i), die + 1)
            }
            Action::MoveNobleCombined { noble: i, distance } => {
                format!("Move {} by {distance} (both dice)", noble(*i))
            }
            Action::MedalMoveNoble { noble: i, delta } => {
                format!("Spend a medal to move {} by {delta:+}", noble(*i))
            }
            Action::ProduceEvent { program: id, medals } => {
                let total = self
                    .current_player()
                    .and_then(|p| self.event_preview(p, *id, *medals))
                    .map_or(0, |pv| pv.breakdown.total());
                if *medals > 0 {
                    format!("Produce {} with {medals} medals: {total} spectators", program(*id))
                } else {
                    format!("Produce {}: {total} spectators", program(*id))
                }
            }
            Action::SkipEvent => "Produce no event".into(),
            Action::DiscardAsset { asset } => format!("Discard a {asset}"),
            Action::RequestAsset { asset } => format!("Request a {asset}"),
            Action::ExchangeMedalForCoins => {
                format!("Exchange a medal for {} coins", self.rules().medals.coin_value)
            }
            Action::ExchangeSpecialEventForMedal => "Exchange a SpecialEvent token for a medal".into(),
        }
    }
}
