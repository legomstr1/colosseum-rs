//! Auction valuation specialist bot.
//!
//! Every holding is valued by the expected final event score it supports.
//! Lots are priced by their marginal gain to me (plus a share of what they
//! would give opponents), and bids stop at that price converted to coins.

mod value;

use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Asset, Game, Phase, PlayerId, Side};
use value::{Eval, Hyp};

pub use value::Params;

/// Working state of the noble-moving search.
struct DiceState<'a> {
    game: &'a Game,
    me: PlayerId,
    pr: &'a colosseum_engine::Producing,
    pos: Vec<u8>,
    used: Vec<bool>,
    moved: Vec<usize>,
    medals_earned: u32,
    medals_supply: u32,
    base: f64,
}

pub struct AuctioneerAgent {
    prm: Params,
    /// Trading-turn bookkeeping: `(round, turn)` key, offers made, and the
    /// price level reached for each `(partner, asset)`.
    trade_key: (u32, usize, usize),
    trade_offers: u32,
    ladder: std::collections::HashMap<(usize, Asset), usize>,
}

impl AuctioneerAgent {
    pub fn new(_seed: u64) -> Self {
        Self::with_params(Params::default())
    }

    pub fn with_params(prm: Params) -> Self {
        Self {
            prm,
            trade_key: (0, 0, 0),
            trade_offers: 0,
            ladder: Default::default(),
        }
    }
}

fn first_of(legal: &[Action], want: &[Action]) -> Option<Action> {
    want.iter().find(|a| legal.contains(a)).cloned()
}

impl AuctioneerAgent {
    // ---- Investing -------------------------------------------------------

    /// Investments the player could make with an investment in hand.
    fn invest_candidates(&self, game: &Game, me: PlayerId) -> Vec<Action> {
        let rules = game.rules();
        let p = game.player(me);
        let mut out = Vec::new();
        for &program in game.programs_for_sale() {
            if game.config().event(program).is_some_and(|e| e.cost <= p.coins) {
                out.push(Action::BuyProgram { program });
            }
        }
        if p.arena.len < rules.arena.max_squares && p.coins >= rules.arena.expansion_cost {
            for side in [Side::Left, Side::Right] {
                let sq = p.arena.expansion_square(side, rules.board.size);
                if game.arena_owner(sq).is_none() {
                    out.push(Action::ExpandArena { side });
                }
            }
        }
        if game.supply().season_tickets > 0 && p.coins >= rules.investing.season_ticket_cost {
            out.push(Action::BuySeasonTicket);
        }
        if !p.loge && game.supply().emperor_loges > 0 && p.coins >= rules.investing.emperor_loge_cost {
            out.push(Action::BuildLoge);
        }
        out
    }

    /// The best investment among `cands`, with its gain in points.
    fn best_investment(&self, game: &Game, me: PlayerId, cands: &[Action]) -> Option<(f64, Action)> {
        let ev = Eval::new(game, &self.prm);
        let base = Hyp::of(game.player(me));
        let base_worth = ev.worth(me, &base);
        let rules = game.rules();
        let mut best: Option<(f64, Action)> = None;
        for a in cands {
            let mut h = base.clone();
            match a {
                Action::BuyProgram { program } => {
                    let Some(def) = game.config().event(*program) else { continue };
                    h.coins = h.coins.saturating_sub(def.cost);
                    h.programs.push((*program, false));
                }
                Action::ExpandArena { .. } => {
                    h.coins = h.coins.saturating_sub(rules.arena.expansion_cost);
                    h.arena_len += 1;
                }
                Action::BuySeasonTicket => {
                    h.coins = h.coins.saturating_sub(rules.investing.season_ticket_cost);
                    h.tickets += 1;
                }
                Action::BuildLoge => {
                    h.coins = h.coins.saturating_sub(rules.investing.emperor_loge_cost);
                    h.loge = true;
                }
                _ => continue,
            }
            let gain = ev.worth(me, &h) - base_worth;
            if best.as_ref().is_none_or(|(g, _)| gain > *g) {
                best = Some((gain, a.clone()));
            }
        }
        best
    }

    fn invest(&self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let remaining = match game.phase() {
            Phase::Investing { remaining, .. } => *remaining,
            _ => 0,
        };
        if remaining > 0 {
            if let Some((gain, a)) = self.best_investment(game, me, legal)
                && gain > self.prm.invest_min
            {
                return a;
            }
            return Action::EndInvesting;
        }
        // No investment left: an extra one (special event token, medals)
        // is worth it if something good could be bought with it.
        let cands = self.invest_candidates(game, me);
        if let Some((gain, _)) = self.best_investment(game, me, &cands) {
            if legal.contains(&Action::PlaySpecialEvent) && gain > self.prm.extra_min {
                return Action::PlaySpecialEvent;
            }
            if legal.contains(&Action::SpendMedalsForInvestment) && gain > self.prm.medal_extra_min {
                return Action::SpendMedalsForInvestment;
            }
        }
        Action::EndInvesting
    }

    // ---- Acquiring -------------------------------------------------------

    /// `(my gain, denial gain)` in points for each market.
    fn lot_values(&self, game: &Game, me: PlayerId) -> Vec<(f64, f64)> {
        let ev = Eval::new(game, &self.prm);
        game.markets()
            .iter()
            .map(|lot| {
                if lot.is_empty() {
                    return (0.0, 0.0);
                }
                let mine = ev.lot_delta(me, lot);
                let deny = game
                    .players()
                    .iter()
                    .filter(|p| p.id != me)
                    .map(|p| ev.lot_delta(p.id, lot))
                    .fold(0.0f64, f64::max);
                (mine, deny.max(0.0))
            })
            .collect()
    }

    fn acquire(&self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Acquiring(acq) = game.phase() else {
            return legal[0].clone();
        };
        let ev = Eval::new(game, &self.prm);
        let lambda = ev.lambda.max(1e-6);
        let _ = lambda;
        let min_bid = game.rules().acquiring.minimum_opening_bid;
        let coins = game.player(me).coins;
        let vals = self.lot_values(game, me);
        let total = |m: usize| vals[m].0 + self.prm.deny * vals[m].1;

        match &acq.auction {
            None => {
                let best = (0..vals.len())
                    .filter(|&m| legal.contains(&Action::StartAuction { market: m, bid: min_bid }))
                    .max_by(|&a, &b| total(a).total_cmp(&total(b)));
                match best {
                    Some(m) if ev.coins_for_pts(total(m), coins as f64) >= min_bid as f64 => Action::StartAuction { market: m, bid: min_bid },
                    _ => Action::PassInitiative,
                }
            }
            Some(auction) => {
                let m = auction.market;
                let alt = (0..vals.len())
                    .filter(|&j| j != m)
                    .map(|j| vals[j].0)
                    .fold(0.0f64, f64::max);
                let alt_surplus = (alt - lambda * min_bid as f64).max(0.0);
                let ceil_pts = total(m) - self.prm.alt_frac * alt_surplus;
                let ceil = (ev.coins_for_pts(ceil_pts, coins as f64) as u32).min(coins.saturating_sub(self.prm.reserve));
                let bid = Action::Bid {
                    amount: auction.high_bid + 1,
                };
                if auction.high_bid < ceil && legal.contains(&bid) {
                    bid
                } else {
                    Action::PassBid
                }
            }
        }
    }

    // ---- Trading ---------------------------------------------------------

    /// Buys tokens I value from opponents for coins, raising the price a
    /// little after each (presumed) refusal.
    fn propose(&mut self, game: &Game, me: PlayerId) -> Action {
        let Phase::Trading { turn, .. } = game.phase() else {
            return Action::EndTrading;
        };
        let key = (game.round(), *turn, game.history().len());
        if self.trade_key.0 != key.0 || self.trade_key.1 != key.1 || key.2 < self.trade_key.2 {
            self.trade_offers = 0;
            self.ladder.clear();
        }
        self.trade_key = key;
        if self.trade_offers >= self.prm.max_offers {
            return Action::EndTrading;
        }
        let ev = Eval::new(game, &self.prm);
        let h = Hyp::of(game.player(me));
        let base = ev.potential(me, &h);
        let coins = h.coins;
        let mut best: Option<(f64, Action, (usize, Asset))> = None;
        for q in game.players().iter().filter(|q| q.id != me) {
            for (asset, _) in q.assets.iter().filter(|(a, _)| a.is_tradeable()) {
                let mut h2 = h.clone();
                h2.assets.add(asset, 1);
                let mut value = ev.potential(me, &h2) - base;
                // Taking it from them also hurts them.
                let hq = Hyp::of(q);
                let mut hq2 = hq.clone();
                hq2.assets.remove(asset, 1);
                value += self.prm.deny * (ev.potential(q.id, &hq) - ev.potential(q.id, &hq2)).max(0.0);
                if value <= 0.5 {
                    continue;
                }
                let level = self.ladder.get(&(q.id.0, asset)).copied().unwrap_or(0);
                let Some(&price) = self.prm.prices.get(level) else { continue };
                if price > coins {
                    continue;
                }
                let cost = ev.coin_pts(coins as f64) - ev.coin_pts((coins - price) as f64);
                let surplus = value - cost;
                if surplus <= 0.0 {
                    continue;
                }
                let mut terms = colosseum_engine::TradeTerms {
                    give_coins: price,
                    ..Default::default()
                };
                terms.receive.add(asset, 1);
                let action = Action::ProposeTrade { to: q.id, terms };
                if best.as_ref().is_none_or(|(s, _, _)| surplus > *s) && game.is_legal(&action) {
                    best = Some((surplus, action, (q.id.0, asset)));
                }
            }
        }
        match best {
            Some((_, action, k)) => {
                *self.ladder.entry(k).or_insert(0) += 1;
                self.trade_offers += 1;
                action
            }
            None => Action::EndTrading,
        }
    }

    fn respond_to_offer(&self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Trading { offer: Some(offer), .. } = game.phase() else {
            return Action::EndTrading;
        };
        if !legal.contains(&Action::AcceptTrade) {
            return Action::RejectTrade;
        }
        let ev = Eval::new(game, &self.prm);
        let before = Hyp::of(game.player(me));
        let mut after = before.clone();
        // Terms are from the proposer's view: I receive `give`, hand over `receive`.
        after.assets.add_all(&offer.terms.give);
        if !after.assets.remove_all(&offer.terms.receive) {
            return Action::RejectTrade;
        }
        after.coins = after.coins + offer.terms.give_coins - offer.terms.receive_coins.min(after.coins + offer.terms.give_coins);
        let gain = ev.worth(me, &after) - ev.worth(me, &before);
        // The partner presumably gains too; only take clearly good deals.
        if gain > 1.0 {
            Action::AcceptTrade
        } else {
            Action::RejectTrade
        }
    }

    // ---- Producing -------------------------------------------------------

    fn noble_sum(&self, game: &Game, me: PlayerId, pos: &[u8]) -> f64 {
        let size = game.rules().board.size;
        let arena = game.player(me).arena;
        game.nobles()
            .iter()
            .zip(pos)
            .filter(|(_, p)| arena.contains(**p, size))
            .map(|(n, _)| game.rules().noble_value(n.kind) as f64)
            .sum()
    }

    /// Best event total (without nobles) over owned programs.
    fn base_event(&self, game: &Game, me: PlayerId) -> f64 {
        game.player(me)
            .programs
            .iter()
            .filter_map(|p| game.event_preview(me, p.id, 0))
            .map(|pv| (pv.breakdown.total() - pv.breakdown.nobles) as f64)
            .fold(0.0, f64::max)
    }

    /// Best total over every way of using the remaining dice, and the first
    /// move of that line.
    fn dice_search(&self, st: &mut DiceState, depth: usize, out_first: &mut Option<Action>) -> f64 {
        let game = st.game;
        if st.used.iter().all(|u| *u) {
            return st.base
                + self.noble_sum(game, st.me, &st.pos)
                + st.medals_earned as f64 * game.rules().scoring.medal_bonus as f64;
        }
        let size = game.rules().board.size as i32;
        let mut best = f64::NEG_INFINITY;
        let mut best_a: Option<Action> = None;
        let mut candidates: Vec<(Action, Vec<usize>)> = Vec::new();
        let mut seen: Vec<&colosseum_engine::DieFace> = Vec::new();
        for (die, face) in st.pr.dice.iter().enumerate() {
            if st.used[die] || seen.contains(&face) {
                continue;
            }
            seen.push(face);
            for noble in (0..st.pos.len()).filter(|n| !st.moved.contains(n)) {
                for &distance in face.values() {
                    candidates.push((Action::MoveNoble { noble, die, distance }, vec![die]));
                }
            }
        }
        if st.pr.dice.len() == 2 && st.moved.is_empty() && st.used.iter().all(|u| !*u) {
            for noble in 0..st.pos.len() {
                for distance in st.pr.combined_distances() {
                    candidates.push((Action::MoveNobleCombined { noble, distance }, vec![0, 1]));
                }
            }
        }
        for (action, dice_used) in candidates {
            let (noble, distance) = match &action {
                Action::MoveNoble { noble, distance, .. } => (*noble, *distance),
                Action::MoveNobleCombined { noble, distance } => (*noble, *distance),
                _ => continue,
            };
            let old = st.pos[noble];
            let new = ((old as i32 + distance as i32).rem_euclid(size)) as u8;
            let earns = game.rules().is_resting_place(new) && st.medals_earned < st.medals_supply;
            st.pos[noble] = new;
            let saved_used = st.used.clone();
            for d in &dice_used {
                st.used[*d] = true;
            }
            st.moved.push(noble);
            st.medals_earned += earns as u32;
            let v = self.dice_search(st, depth + 1, &mut None);
            st.medals_earned -= earns as u32;
            st.moved.pop();
            st.used = saved_used;
            st.pos[noble] = old;
            if v > best {
                best = v;
                best_a = Some(action);
            }
        }
        if depth == 0 {
            *out_first = best_a;
        }
        best
    }

    fn produce(&self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Producing(pr) = game.phase() else {
            return legal[0].clone();
        };
        let ev = Eval::new(game, &self.prm);
        let rules = game.rules();
        let last_round = game.round() >= rules.num_rounds;

        if legal.iter().any(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. })) {
            let mut st = DiceState {
                game,
                me,
                pr,
                pos: game.nobles().iter().map(|n| n.position).collect(),
                used: pr.used.clone(),
                moved: pr.moved.clone(),
                medals_earned: 0,
                medals_supply: game.supply().medals,
                base: self.base_event(game, me),
            };
            let mut first = None;
            self.dice_search(&mut st, 0, &mut first);
            if let Some(a) = first.filter(|a| legal.contains(a)) {
                return a;
            }
            return legal
                .iter()
                .find(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }))
                .cloned()
                .unwrap_or_else(|| legal[0].clone());
        }

        let player = game.player(me);
        if last_round && legal.contains(&Action::ExchangeSpecialEventForMedal) {
            return Action::ExchangeSpecialEventForMedal;
        }

        // Spend medals to pull nobles into my arena when it pays more than
        // the medal is worth.
        if player.medals > 0 {
            let pos: Vec<u8> = game.nobles().iter().map(|n| n.position).collect();
            let cur = self.noble_sum(game, me, &pos);
            let mut best: Option<(f64, Action)> = None;
            let size = rules.board.size as i32;
            for a in legal {
                if let Action::MedalMoveNoble { noble, delta } = a {
                    let mut p2 = pos.clone();
                    p2[*noble] = ((p2[*noble] as i32 + *delta as i32).rem_euclid(size)) as u8;
                    let gain = self.noble_sum(game, me, &p2) - cur;
                    if best.as_ref().is_none_or(|(g, _)| gain > *g) {
                        best = Some((gain, a.clone()));
                    }
                }
            }
            if let Some((gain, a)) = best
                && gain >= ev.medal_pts() + 1.0
            {
                return a;
            }
        }

        // Pick the event.
        let medals = if last_round { player.medals } else { 0 };
        let mut best: Option<(f64, Action)> = None;
        for a in legal {
            let Action::ProduceEvent { program, medals: m } = a else { continue };
            if *m != medals {
                continue;
            }
            let Some(pv) = game.event_preview(me, *program, *m) else { continue };
            let spectators = pv.breakdown.total() as f64;
            let loss = if last_round { 0.0 } else { self.discard_loss(game, me, &ev, &pv.used) };
            let score = spectators - loss;
            if best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, a.clone()));
            }
        }
        match best {
            Some((s, a)) if s > 0.0 => a,
            _ => Action::SkipEvent,
        }
    }

    /// Cheapest token to throw away after producing with `used`.
    fn discard_loss(&self, game: &Game, me: PlayerId, ev: &Eval, used: &colosseum_engine::Assets) -> f64 {
        let h = Hyp::of(game.player(me));
        let base = ev.potential(me, &h);
        used.iter()
            .filter(|(a, _)| h.assets.count(*a) > 0)
            .map(|(a, _)| {
                let mut h2 = h.clone();
                h2.assets.remove(a, 1);
                base - ev.potential(me, &h2)
            })
            .fold(f64::INFINITY, f64::min)
            .clamp(0.0, 1000.0)
    }

    // ---- Closing ---------------------------------------------------------

    fn close(&self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let ev = Eval::new(game, &self.prm);
        let h = Hyp::of(game.player(me));
        let base = ev.potential(me, &h);
        let mut best: Option<(f64, Action)> = None;
        for a in legal {
            let score = match a {
                Action::DiscardAsset { asset } => {
                    let mut h2 = h.clone();
                    h2.assets.remove(*asset, 1);
                    ev.potential(me, &h2) - base
                }
                Action::RequestAsset { asset } => {
                    let mut h2 = h.clone();
                    h2.assets.add(*asset, 1);
                    ev.potential(me, &h2) - base
                }
                _ => continue,
            };
            if best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, a.clone()));
            }
        }
        best.map(|(_, a)| a).unwrap_or_else(|| legal[0].clone())
    }
}

impl Agent for AuctioneerAgent {
    fn name(&self) -> String {
        "auctioneer".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
        let choice = match game.phase() {
            Phase::Investing { .. } => self.invest(game, me, legal),
            Phase::Acquiring(_) => self.acquire(game, me, legal),
            Phase::Trading { offer: None, .. } => self.propose(game, me),
            Phase::Trading { .. } => self.respond_to_offer(game, me, legal),
            Phase::Producing(_) => self.produce(game, me, legal),
            Phase::Closing(_) => self.close(game, me, legal),
            Phase::GameOver => legal[0].clone(),
        };
        let ok = match &choice {
            Action::ProposeTrade { .. } => game.is_legal(&choice),
            _ => legal.contains(&choice),
        };
        if ok {
            return choice;
        }
        first_of(legal, &[Action::EndInvesting, Action::PassBid, Action::PassInitiative, Action::EndTrading, Action::RejectTrade, Action::SkipEvent])
            .unwrap_or_else(|| legal[0].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colosseum_engine::agent::{play_out, GreedyAgent, RandomAgent};
    use colosseum_engine::GameConfig;
    use std::sync::Arc;

    #[test]
    fn plays_full_games() {
        let config = Arc::new(GameConfig::embedded());
        for n in 3..=5 {
            for seed in 0..6u64 {
                let mut game = Game::new(config.clone(), n, seed).unwrap();
                let mut agents: Vec<Box<dyn Agent>> = (0..n)
                    .map(|i| match i % 3 {
                        0 => Box::new(AuctioneerAgent::new(seed)) as Box<dyn Agent>,
                        1 => Box::new(GreedyAgent::new(seed)),
                        _ => Box::new(RandomAgent::new(seed)),
                    })
                    .collect();
                play_out(&mut game, &mut agents);
                assert!(game.is_over());
            }
        }
    }
}
