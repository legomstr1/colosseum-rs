//! Active trade negotiator bot.
//!
//! The baseline bots never propose trades. This one prices every trade by how
//! much it changes the worth of both positions (see `value`), proposes swaps
//! and coin deals that help it while leaving the partner plausibly better
//! off, and answers or counters offers the same way. The other phases use
//! solid one-step heuristics on the same valuation.

mod nobles;
mod trade;
mod value;

use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Action, Asset, Game, Phase, PlayerId, TradeTerms};
use nobles::NobleSim;
use trade::{Pair, best_offer, rho};
use value::{Weights, tune, star_holder, stars_now, weights, worth, worth_now};

/// Heuristic limits on my own negotiating, inside the rules' caps.
const MAX_OFFERS_PER_PARTNER: u32 = 6;
const MAX_OFFERS_PER_TURN: u32 = 12;
const MAX_COUNTERS: u32 = 2;
const MAX_REJECTIONS_PER_PARTNER: usize = 4;

pub struct TraderAgent {
    rng: Rng,
    /// `(round, trading turn)` the negotiation memory below belongs to.
    turn_key: (u32, usize),
    /// Terms I made this turn that were turned down, by partner.
    rejected: Vec<(PlayerId, TradeTerms)>,
    /// My last offer or counter, to learn whether it was rejected.
    pending: Option<(PlayerId, TradeTerms)>,
    /// Whether to make offers and counter-offers (off for ablation tests).
    negotiate: bool,
}

impl TraderAgent {
    /// A copy that never proposes or counters (it still accepts good offers).
    pub fn passive(mut self) -> Self {
        self.negotiate = false;
        self
    }

    pub fn new(seed: u64) -> Self {
        Self {
            negotiate: true,
            rng: Rng::new(seed),
            turn_key: (0, usize::MAX),
            rejected: Vec::new(),
            pending: None,
        }
    }
}

/// Rounds left including the current one.
fn rounds_left(game: &Game) -> u32 {
    (game.rules().num_rounds + 1).saturating_sub(game.round()).max(1)
}

fn is_last_round(game: &Game) -> bool {
    game.round() >= game.rules().num_rounds
}

/// Change in worth for `me` from gaining `tokens` (as won at auction or
/// donated), including star awards and Emperor tokens turning into medals.
fn gain_from_tokens(game: &Game, w: &Weights, me: PlayerId, tokens: &[Asset]) -> f64 {
    let p = game.player(me);
    let mut assets = p.assets.clone();
    let mut extra = 0.0;
    for &t in tokens {
        match t {
            Asset::Emperor => {
                if game.supply().medals > 0 {
                    extra += if is_last_round(game) { 3.0 } else { 2.2 };
                }
            }
            Asset::SpecialEvent => {
                extra += 2.0;
                assets.add(t, 1);
            }
            _ => assets.add(t, 1),
        }
    }
    let mut stars = stars_now(game, me);
    for &a in &game.rules().stars.assets {
        if assets.count(a) != p.assets.count(a) {
            stars[a.index()] = star_holder(game, a, &[(me, assets.count(a))]) == Some(me);
        }
    }
    worth(game, w, me, &assets, p.coins, &stars, p.arena.len, None) - worth_now(game, w, me) + extra
}

fn is_dice_move(a: &Action) -> bool {
    matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. })
}

impl TraderAgent {
    fn pick_best<T: Clone>(&mut self, scored: Vec<(f64, T)>) -> Option<T> {
        let best = scored.iter().map(|(s, _)| *s).fold(f64::NEG_INFINITY, f64::max);
        let top: Vec<T> = scored
            .into_iter()
            .filter(|(s, _)| (*s - best).abs() < 1e-9)
            .map(|(_, a)| a)
            .collect();
        self.rng.choose(&top).cloned()
    }

    // ---- Phase 1: investing --------------------------------------------

    fn invest(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let w = weights(game);
        let rules = game.rules();
        let p = game.player(me);
        let rr = rounds_left(game);
        let base = worth_now(game, &w, me);
        let stars = stars_now(game, me);
        let produced = p.programs.iter().filter(|q| q.produced).count() as u32;
        let k = p.programs.len() as u32;
        let others = |n: u32| n.saturating_sub(1).min(produced + rr - 1);
        let remaining = match game.phase() {
            Phase::Investing { remaining, .. } => *remaining,
            _ => 0,
        };

        let mut options: Vec<(f64, Action)> = Vec::new();
        for a in legal {
            let v = match a {
                Action::BuyProgram { program } => {
                    let cost = game.config().event(*program).map_or(0, |e| e.cost);
                    let after = worth(
                        game,
                        &w,
                        me,
                        &p.assets,
                        p.coins - cost,
                        &stars,
                        p.arena.len,
                        Some(*program),
                    );
                    let bonus = (others(k + 1) - others(k)) as f64 * rules.scoring.previous_program_bonus as f64 * tune("progbonus", 1.0);
                    after - base + bonus
                }
                Action::ExpandArena { .. } => {
                    let cost = rules.arena.expansion_cost;
                    worth(game, &w, me, &p.assets, p.coins - cost, &stars, p.arena.len + 1, None) - base + tune("arena", 3.5)
                }
                Action::BuySeasonTicket => {
                    rules.scoring.season_ticket_bonus as f64 - rules.investing.season_ticket_cost as f64 * w.coin
                }
                Action::BuildLoge => {
                    tune("loge", 4.0) * rr.min(3) as f64 - rules.investing.emperor_loge_cost as f64 * w.coin
                }
                _ => continue,
            };
            options.push((v, a.clone()));
        }
        if remaining == 1 && legal.contains(&Action::PlaySpecialEvent) {
            let good = options.iter().filter(|(v, _)| *v > 2.5).count();
            if good >= 2 {
                return Action::PlaySpecialEvent;
            }
        }
        options.sort_by(|a, b| b.0.total_cmp(&a.0));
        match options.first() {
            Some((v, a)) if *v > tune("invmin", 0.3) => a.clone(),
            _ => Action::EndInvesting,
        }
    }

    // ---- Phase 2: acquiring --------------------------------------------

    fn acquire(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Acquiring(acq) = game.phase() else {
            unreachable!()
        };
        let w = weights(game);
        let coin_w = w.coin.max(0.01);
        let coins = game.player(me).coins;
        match &acq.auction {
            None => {
                let min = game.rules().acquiring.minimum_opening_bid;
                let best = (0..game.markets().len())
                    .filter(|m| legal.contains(&Action::StartAuction { market: *m, bid: min }))
                    .map(|m| (gain_from_tokens(game, &w, me, &game.markets()[m]) - coin_w * min as f64, m))
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                match best {
                    Some((net, market)) if net > tune("open", 0.0) => Action::StartAuction { market, bid: min },
                    _ => Action::PassInitiative,
                }
            }
            Some(auction) => {
                let gain = gain_from_tokens(game, &w, me, &game.markets()[auction.market]);
                let max_pay = (tune("shade", 0.85) * gain / coin_w).floor().max(0.0) as u32;
                let bid = Action::Bid {
                    amount: auction.high_bid + 1,
                };
                if auction.high_bid < max_pay && auction.high_bid < coins && legal.contains(&bid) {
                    bid
                } else {
                    Action::PassBid
                }
            }
        }
    }

    // ---- Phase 3: trading ----------------------------------------------

    fn trade(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Trading {
            turn,
            offers,
            counters,
            offer,
        } = game.phase()
        else {
            unreachable!()
        };
        let key = (game.round(), *turn);
        if let Some((to, terms)) = self.pending.take()
            && self.turn_key == key
            && matches!(game.history().last(), Some(Action::RejectTrade))
        {
            self.rejected.push((to, terms));
        }
        if self.turn_key != key {
            self.turn_key = key;
            self.rejected.clear();
        }
        let w = weights(game);

        match offer {
            Some(o) => {
                if !legal.contains(&Action::AcceptTrade) {
                    return Action::RejectTrade;
                }
                let proposer = o.from;
                let e = Pair::new(game, &w, proposer, me).eval(&o.terms);
                let (mine, theirs) = (e.to, e.from);
                if mine >= tune("accme", 0.4) && mine - rho() * theirs.max(0.0) >= 0.2 {
                    return Action::AcceptTrade;
                }
                let active = game.seat(*turn);
                let partner = if o.from == active { o.to } else { o.from };
                if self.negotiate
                    && counters[partner.0] < MAX_COUNTERS.min(game.rules().trading.max_counter_offers_per_partner)
                    && let Some(c) = best_offer(game, &w, me, proposer, 0.5, &[], true)
                    && c.mine >= (mine + 0.3).max(0.8)
                {
                    self.pending = Some((proposer, c.terms.clone()));
                    return Action::CounterTrade { terms: c.terms };
                }
                Action::RejectTrade
            }
            None => {
                let total: u32 = offers.iter().sum();
                if total >= tune("turncap", MAX_OFFERS_PER_TURN as f64) as u32 || !self.negotiate {
                    return Action::EndTrading;
                }
                let cap = (tune("cap", MAX_OFFERS_PER_PARTNER as f64) as u32).min(game.rules().trading.max_offers_per_partner);
                let mut best: Option<(f64, PlayerId, trade::Candidate)> = None;
                for q in game.players().iter().map(|p| p.id).filter(|&q| q != me) {
                    let turned_down: Vec<TradeTerms> = self
                        .rejected
                        .iter()
                        .filter(|(to, _)| *to == q)
                        .map(|(_, t)| t.clone())
                        .collect();
                    if offers[q.0] >= cap || turned_down.len() >= tune("maxrej", MAX_REJECTIONS_PER_PARTNER as f64) as usize {
                        continue;
                    }
                    let their_min = tune("tmin", 0.1) + tune("tstep", 0.3) * turned_down.len() as f64;
                    if let Some(c) = best_offer(game, &w, me, q, their_min, &turned_down, false) {
                        let s = c.mine - tune("objw", 0.4) * c.theirs.max(0.0);
                        if best.as_ref().is_none_or(|b| s > b.0) {
                            best = Some((s, q, c));
                        }
                    }
                }
                match best {
                    Some((_, to, c)) => {
                        self.pending = Some((to, c.terms.clone()));
                        Action::ProposeTrade { to, terms: c.terms }
                    }
                    None => Action::EndTrading,
                }
            }
        }
    }

    // ---- Phase 4: producing --------------------------------------------

    fn produce(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let last = is_last_round(game);
        if last && legal.contains(&Action::ExchangeSpecialEventForMedal) {
            return Action::ExchangeSpecialEventForMedal;
        }
        let medal_value = if last { 3.0 } else { 2.0 };
        let sim = NobleSim::new(game, me, medal_value);
        if legal.iter().any(is_dice_move) {
            let scored = sim.score_dice_moves(legal);
            if let Some(a) = self.pick_best(scored) {
                return a;
            }
        }
        if let Some((_, a)) = sim.best_medal_move(legal) {
            return a;
        }

        let medals = if last { game.player(me).medals } else { 0 };
        let produce: Vec<(f64, Action)> = legal
            .iter()
            .filter_map(|a| match a {
                Action::ProduceEvent { program, medals: m } if *m == medals => {
                    let pv = game.event_preview(me, *program, *m)?;
                    let fresh = game.player(me).programs.iter().any(|q| q.id == *program && !q.produced);
                    let bonus = if !last && fresh { 3.0 } else { 0.0 };
                    Some((pv.breakdown.total() as f64 + bonus, a.clone()))
                }
                _ => None,
            })
            .collect();
        self.pick_best(produce).unwrap_or(Action::SkipEvent)
    }

    // ---- Phase 5: closing ceremonies -----------------------------------

    fn close(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let w = weights(game);
        let p = game.player(me);
        let stars = stars_now(game, me);
        let scored: Vec<(f64, Action)> = legal
            .iter()
            .filter_map(|a| match a {
                Action::DiscardAsset { asset } => {
                    let mut left = p.assets.clone();
                    left.remove(*asset, 1);
                    Some((worth(game, &w, me, &left, p.coins, &stars, p.arena.len, None), a.clone()))
                }
                Action::RequestAsset { asset } => Some((gain_from_tokens(game, &w, me, &[*asset]), a.clone())),
                _ => None,
            })
            .collect();
        self.pick_best(scored).unwrap_or_else(|| legal[0].clone())
    }

    /// A move that is always legal for the phase, used if a choice fails.
    fn fallback(legal: &[Action]) -> Action {
        const SAFE: [Action; 6] = [
            Action::PassBid,
            Action::PassInitiative,
            Action::EndInvesting,
            Action::EndTrading,
            Action::RejectTrade,
            Action::SkipEvent,
        ];
        SAFE.iter()
            .find(|a| legal.contains(a))
            .cloned()
            .unwrap_or_else(|| legal[0].clone())
    }
}

impl Agent for TraderAgent {
    fn name(&self) -> String {
        "trader".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
        let action = match game.phase() {
            Phase::Investing { .. } => self.invest(game, me, legal),
            Phase::Acquiring(_) => self.acquire(game, me, legal),
            Phase::Trading { .. } => self.trade(game, me, legal),
            Phase::Producing(_) => self.produce(game, me, legal),
            Phase::Closing(_) => self.close(game, me, legal),
            Phase::GameOver => return legal[0].clone(),
        };
        if legal.contains(&action) || game.is_legal(&action) {
            action
        } else {
            self.pending = None;
            Self::fallback(legal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colosseum_engine::GameConfig;
    use std::sync::Arc;

    #[test]
    fn plays_full_games_legally() {
        let config = Arc::new(GameConfig::embedded());
        for (n, seed) in [(3usize, 1u64), (4, 2), (5, 3)] {
            let mut game = Game::new(config.clone(), n, seed).unwrap();
            let mut agents: Vec<TraderAgent> = (0..n).map(|i| TraderAgent::new(seed + i as u64)).collect();
            let mut steps = 0;
            while let Some(p) = game.current_player() {
                let legal = game.legal_actions();
                let a = agents[p.0].choose_action(&game, &legal);
                game.apply(a).expect("legal");
                steps += 1;
                assert!(steps < 20_000, "game did not terminate");
            }
            assert!(game.is_over());
        }
    }

    #[test]
    fn negotiates_and_trades_happen() {
        let config = Arc::new(GameConfig::embedded());
        let mut accepted = 0;
        for seed in 0..20u64 {
            let mut game = Game::new(config.clone(), 4, seed).unwrap();
            let mut agents: Vec<TraderAgent> = (0..4).map(|i| TraderAgent::new(seed * 10 + i)).collect();
            while let Some(p) = game.current_player() {
                let legal = game.legal_actions();
                let a = agents[p.0].choose_action(&game, &legal);
                game.apply(a).expect("legal");
            }
            accepted += game.history().iter().filter(|a| matches!(a, Action::AcceptTrade)).count();
        }
        assert!(accepted > 0, "traders never agreed on a trade");
    }
}
