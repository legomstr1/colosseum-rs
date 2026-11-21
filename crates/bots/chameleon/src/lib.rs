//! Chameleon: an opponent-aware, adaptive Colosseum player.
//!
//! Every seat's holding is valued with one projection of its final-round
//! event (`model`). Chameleon applies that projection to its opponents to
//! see who threatens to win and what each one needs, and uses it to
//! - bid on lots partly to deny the leader what they need,
//! - open the lots where its surplus over the expected competition is best,
//! - price coin-for-token offers and token swaps against what the partner
//!   would lose, learning per opponent how cheaply they sell,
//! - accept only trades whose gain to it beats the partner's gain weighted
//!   by how threatening the partner is (never feeding the leader), and
//!   never sell spare tokens to rivals,
//! - steer nobles away from threatening rivals' arenas in the final round,
//! - and ask the podium winner for the token that hurts them most.

mod model;
mod produce;

use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Asset, Assets, Game, Phase, PlayerId, Side, TradeTerms};
use model::{Hyp, Model, Req, Timing, build_reqs, threat_weights};

pub use model::Params;

const EPS: f64 = 0.05;

pub struct ChameleonAgent {
    prm: Params,
    reqs: Vec<Req>,
    /// `(round, trading turn)` the negotiation memory belongs to.
    trade_key: (u32, usize),
    proposals: u32,
    /// Offers turned down this turn.
    rejected: Vec<TradeKey>,
    /// My last proposal, to learn how it was answered.
    pending: Option<TradeKey>,
    /// Per-opponent price scale for buying tokens, learned from answers.
    scale: Vec<f64>,
    /// History length at my last decision (detects a new game).
    seen: usize,
}

/// `(partner, token I receive, token I give)`.
type TradeKey = (usize, Option<Asset>, Option<Asset>);

impl ChameleonAgent {
    pub fn new(_seed: u64) -> Self {
        Self::with_params(Params::default())
    }

    pub fn with_params(prm: Params) -> Self {
        Self {
            prm,
            reqs: Vec::new(),
            trade_key: (0, usize::MAX),
            proposals: 0,
            rejected: Vec::new(),
            pending: None,
            scale: Vec::new(),
            seen: 0,
        }
    }
}

fn safe_fallback(legal: &[Action]) -> Action {
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

// ---- Investing ---------------------------------------------------------------

fn investments(m: &Model, me: PlayerId, h0: &Hyp) -> Vec<(Action, Hyp)> {
    let g = m.g;
    let rules = g.rules();
    let pl = g.player(me);
    let mut out = Vec::new();
    for &program in g.programs_for_sale() {
        let cost = m.def(program).map_or(u32::MAX, |(d, _)| d.cost);
        if h0.coins >= cost {
            let mut h = h0.clone();
            h.coins -= cost;
            h.add_program(program);
            out.push((Action::BuyProgram { program }, h));
        }
    }
    if h0.arena_len < rules.arena.max_squares && h0.coins >= rules.arena.expansion_cost {
        let mut best: Option<(f64, Side)> = None;
        for side in [Side::Left, Side::Right] {
            let sq = pl.arena.expansion_square(side, rules.board.size);
            if g.arena_owner(sq).is_some() {
                continue;
            }
            let mut score = 0.0;
            for n in g.nobles() {
                let dist = (sq as i32 - n.position as i32).rem_euclid(rules.board.size as i32) as f64;
                let v = rules.noble_value(n.kind) as f64;
                if dist == 0.0 {
                    score += v;
                } else if dist <= 6.0 {
                    score += v / (1.0 + dist);
                }
            }
            // Keep room to grow again on the other side.
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, side));
            }
        }
        if let Some((_, side)) = best {
            let mut h = h0.clone();
            h.coins -= rules.arena.expansion_cost;
            h.arena_len += 1;
            out.push((Action::ExpandArena { side }, h));
        }
    }
    if g.supply().season_tickets > 0 && h0.coins >= rules.investing.season_ticket_cost {
        let mut h = h0.clone();
        h.coins -= rules.investing.season_ticket_cost;
        h.tickets += 1;
        out.push((Action::BuySeasonTicket, h));
    }
    if !h0.loge && g.supply().emperor_loges > 0 && h0.coins >= rules.investing.emperor_loge_cost {
        let mut h = h0.clone();
        h.coins -= rules.investing.emperor_loge_cost;
        h.loge = true;
        out.push((Action::BuildLoge, h));
    }
    out
}

fn best_investment(m: &Model, me: PlayerId, h0: &Hyp, t: &Timing, base: f64) -> Option<(Action, f64)> {
    let mut after = *t;
    after.slots = t.slots.saturating_sub(1);
    investments(m, me, h0)
        .into_iter()
        .map(|(a, h)| {
            let v = m.value(me.0, &h, &after) - base;
            (a, v)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

fn invest(m: &Model, me: PlayerId, legal: &[Action], remaining: u32) -> Action {
    let h0 = Hyp::of(m.g, me);
    let t = m.timing(me);
    let base = m.value(me.0, &h0, &t);
    if remaining > 0 {
        if let Some((a, gain)) = best_investment(m, me, &h0, &t, base)
            && gain > EPS
            && legal.contains(&a)
        {
            return a;
        }
    } else {
        let rules = m.g.rules();
        if legal.contains(&Action::PlaySpecialEvent) {
            let mut t2 = t;
            t2.slots += rules.investing.special_event_extra_investments;
            if let Some((_, gain)) = best_investment(m, me, &h0, &t2, base)
                && gain > m.prm.medal_value
            {
                return Action::PlaySpecialEvent;
            }
        }
        if legal.contains(&Action::SpendMedalsForInvestment) {
            let mut h = h0.clone();
            h.medals -= rules.investing.extra_investment_medal_cost;
            let mut t2 = t;
            t2.slots += 1;
            if let Some((_, gain)) = best_investment(m, me, &h, &t2, base)
                && gain > EPS
            {
                return Action::SpendMedalsForInvestment;
            }
        }
    }
    if legal.contains(&Action::ExchangeSpecialEventForMedal) {
        return Action::ExchangeSpecialEventForMedal;
    }
    Action::EndInvesting
}

// ---- Acquiring ---------------------------------------------------------------

/// Points `who` gains from a lot, valued with a discounted future.
fn lot_gain(m: &Model, who: PlayerId, lot: &[Asset]) -> f64 {
    let h0 = Hyp::of(m.g, who);
    let mut t = m.timing(who);
    t.acq_left *= m.prm.kappa;
    let mut h1 = h0.clone();
    h1.add_tokens(lot, m.g.supply().medals);
    m.value(who.0, &h1, &t) - m.value(who.0, &h0, &t)
}

struct LotView {
    /// My cap in coins, including denial.
    cap: f64,
    /// Highest estimated willingness to pay among other bidders.
    rival: f64,
}

fn lot_views(m: &Model, me: PlayerId, won: &[bool]) -> Vec<Option<LotView>> {
    let g = m.g;
    let lambda = m.lambda().max(1e-6);
    let w = threat_weights(m, me);
    g.markets()
        .iter()
        .map(|lot| {
            if lot.is_empty() {
                return None;
            }
            let mine = lot_gain(m, me, lot);
            let mut deny = 0.0f64;
            let mut rival = 0.0f64;
            for (q, wq) in w.iter().enumerate() {
                if q == me.0 || won.get(q).copied().unwrap_or(false) {
                    continue;
                }
                let gq = lot_gain(m, PlayerId(q), lot);
                rival = rival.max(gq * m.prm.rival_scale / lambda);
                deny = deny.max(gq.max(0.0) * wq);
            }
            let pts = mine + m.prm.deny * deny;
            Some(LotView {
                cap: pts * m.prm.pay_frac / lambda,
                rival,
            })
        })
        .collect()
}

fn acquire(m: &Model, me: PlayerId, legal: &[Action]) -> Action {
    let g = m.g;
    let Phase::Acquiring(acq) = g.phase() else {
        return safe_fallback(legal);
    };
    let coins = g.player(me).coins;
    let min = g.rules().acquiring.minimum_opening_bid;
    let views = lot_views(m, me, &acq.won);
    let surplus = |v: &LotView| v.cap - v.rival.max(min as f64);
    match &acq.auction {
        None => {
            let mut best: Option<(f64, usize)> = None;
            for (i, v) in views.iter().enumerate() {
                let Some(v) = v else { continue };
                if v.cap < min as f64 || coins < min {
                    continue;
                }
                let s = surplus(v).max(0.0) + m.prm.open_cap_w * v.cap;
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, i));
                }
            }
            match best {
                Some((_, market)) => Action::StartAuction { market, bid: min },
                None => Action::PassInitiative,
            }
        }
        Some(a) => {
            let Some(Some(v)) = views.get(a.market) else {
                return Action::PassBid;
            };
            // Winning ends my acquiring: count what the best other lot offers.
            let alt = views
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != a.market)
                .filter_map(|(_, x)| x.as_ref().map(surplus))
                .fold(0.0f64, f64::max);
            let cap = (v.cap - m.prm.alt_frac * alt).floor().max(0.0) as u32;
            let cap = cap.min(coins);
            if a.high_bid < cap {
                Action::Bid { amount: a.high_bid + 1 }
            } else {
                Action::PassBid
            }
        }
    }
}

// ---- Trading -------------------------------------------------------------------

fn single(a: Asset) -> Assets {
    let mut x = Assets::new();
    x.add(a, 1);
    x
}

/// Points `who` gains (value incl. coins) if their holding changes by
/// `gain`/`lose` tokens and `coins` (signed).
fn delta(m: &Model, who: PlayerId, t: &Timing, base: f64, gain: &Assets, lose: &Assets, coins: i64) -> Option<f64> {
    let mut h = Hyp::of(m.g, who);
    if !h.assets.remove_all(lose) {
        return None;
    }
    h.assets.add_all(gain);
    let c = h.coins as i64 + coins;
    if c < 0 {
        return None;
    }
    h.coins = c as u32;
    Some(m.value(who.0, &h, t) - base)
}

impl ChameleonAgent {
    fn trade(&mut self, m: &Model, me: PlayerId, legal: &[Action]) -> Action {
        let g = m.g;
        let Phase::Trading { turn, offer, counters, .. } = g.phase() else {
            return safe_fallback(legal);
        };
        let key = (g.round(), *turn);
        if self.trade_key != key {
            self.trade_key = key;
            self.proposals = 0;
            self.rejected.clear();
            self.pending = None;
        }
        if self.scale.len() != g.num_players() {
            self.scale = vec![m.prm.start_scale; g.num_players()];
        }
        if let Some(p) = self.pending.take() {
            let (qi, recv, give) = p;
            match g.history().last() {
                Some(Action::RejectTrade) => {
                    self.rejected.push(p);
                    if recv.is_some() && give.is_none() && self.level(p) >= 2 {
                        self.scale[qi] = (self.scale[qi] * 1.1).min(2.0);
                    }
                }
                Some(Action::AcceptTrade) if recv.is_some() && give.is_none() && self.level(p) == 0 => {
                    self.scale[qi] = (self.scale[qi] * m.prm.scale_down).max(0.3);
                }
                _ => {}
            }
        }
        match offer {
            Some(o) => self.respond(m, me, legal, o, counters),
            None => self.propose(m, me),
        }
    }

    fn respond(&mut self, m: &Model, me: PlayerId, legal: &[Action], o: &colosseum_engine::Offer, counters: &[u32]) -> Action {
        if !legal.contains(&Action::AcceptTrade) {
            return Action::RejectTrade;
        }
        let g = m.g;
        let q = o.from;
        let t = m.timing(me);
        let tq = m.timing(q);
        let base_me = m.value(me.0, &Hyp::of(g, me), &t);
        let base_q = m.value(q.0, &Hyp::of(g, q), &tq);
        let terms = &o.terms;
        let coins_me = terms.give_coins as i64 - terms.receive_coins as i64;
        let Some(mine) = delta(m, me, &t, base_me, &terms.give, &terms.receive, coins_me) else {
            return Action::RejectTrade;
        };
        let theirs = delta(m, q, &tq, base_q, &terms.receive, &terms.give, -coins_me).unwrap_or(0.0);
        let w = threat_weights(m, me);
        let net = mine - w[q.0] * theirs.max(0.0);
        if net > m.prm.accept_min {
            return Action::AcceptTrade;
        }
        // Counter a coins-for-my-tokens offer with a price I'd take.
        let active = match g.phase() {
            Phase::Trading { turn, .. } => g.seat(*turn),
            _ => q,
        };
        let partner = if q == active { me } else { q };
        let lambda = t.lambda.max(1e-6);
        if m.prm.counter > 0.5
            && terms.give.is_empty()
            && !terms.receive.is_empty()
            && terms.receive_coins == 0
            && counters.get(partner.0).copied().unwrap_or(u32::MAX) < 2
            && theirs > 0.0
        {
            let lose_pts = mine - coins_me as f64 * lambda; // token-only change (negative)
            let need = (m.prm.accept_min + 0.5 + w[q.0] * theirs - lose_pts) / lambda;
            let price = need.ceil().max(1.0) as u32;
            if price > terms.give_coins && price <= g.player(q).coins && (price as f64) * lambda < theirs * 0.9 {
                // Counter-offer terms are from my view: I give the tokens, receive coins.
                let ct = TradeTerms {
                    give: terms.receive.clone(),
                    give_coins: 0,
                    receive: Assets::new(),
                    receive_coins: price,
                };
                let a = Action::CounterTrade { terms: ct };
                if g.is_legal(&a) {
                    return a;
                }
            }
        }
        Action::RejectTrade
    }

    fn propose(&mut self, m: &Model, me: PlayerId) -> Action {
        let g = m.g;
        if self.proposals >= m.prm.max_props as u32 {
            return Action::EndTrading;
        }
        let t = m.timing(me);
        let lambda = t.lambda.max(1e-6);
        let h0 = Hyp::of(g, me);
        let base = m.value(me.0, &h0, &t);
        let w = threat_weights(m, me);
        let coins = h0.coins;

        // My marginal values: gains for tokens I lack, losses for ones I hold.
        let mut my_gain = [0.0f64; Asset::COUNT];
        let mut my_loss = [f64::INFINITY; Asset::COUNT];
        for a in Asset::ALL.into_iter().filter(|a| a.is_tradeable()) {
            let mut h1 = h0.clone();
            h1.assets.add(a, 1);
            my_gain[a.index()] = m.value(me.0, &h1, &t) - base;
            if h0.assets.count(a) > 0 {
                let mut h2 = h0.clone();
                h2.assets.remove(a, 1);
                my_loss[a.index()] = (base - m.value(me.0, &h2, &t)).max(0.0);
            }
        }

        let margin = m.prm.trade_margin;
        let mut best: Option<(f64, Action, TradeKey)> = None;
        let consider = |score: f64, terms: TradeTerms, key: TradeKey, best: &mut Option<(f64, Action, TradeKey)>| {
            if score <= m.prm.min_surplus || best.as_ref().is_some_and(|b| score <= b.0) {
                return;
            }
            let action = Action::ProposeTrade { to: PlayerId(key.0), terms };
            if g.is_legal(&action) {
                *best = Some((score, action, key));
            }
        };
        for q in g.players() {
            if q.id == me {
                continue;
            }
            let qi = q.id.0;
            let tq = m.timing(q.id);
            let hq = Hyp::of(g, q.id);
            let base_q = m.value(qi, &hq, &tq);
            let dq = m.prm.deny * w[qi];
            let scale = self.scale.get(qi).copied().unwrap_or(1.0);
            // Their marginal values.
            let mut q_loss = [0.0f64; Asset::COUNT];
            let mut q_gain = [0.0f64; Asset::COUNT];
            let mut wants: Vec<Asset> = Vec::new();
            let mut spares: Vec<Asset> = Vec::new();
            for a in Asset::ALL.into_iter().filter(|a| a.is_tradeable()) {
                if q.assets.count(a) > 0 && my_gain[a.index()] >= 1.0 {
                    let mut h1 = hq.clone();
                    h1.assets.remove(a, 1);
                    q_loss[a.index()] = (base_q - m.value(qi, &h1, &tq)).max(0.0);
                    wants.push(a);
                }
                if h0.assets.count(a) > 0 && my_loss[a.index()] < 1.0 {
                    let mut h1 = hq.clone();
                    h1.assets.add(a, 1);
                    q_gain[a.index()] = (m.value(qi, &h1, &tq) - base_q).max(0.0);
                    if q_gain[a.index()] > 0.5 {
                        spares.push(a);
                    }
                }
            }
            // Buy a token for coins.
            for &a in &wants {
                let key = (qi, Some(a), None);
                let k = self.level(key);
                if k >= 5 {
                    continue;
                }
                let floor = (q_loss[a.index()] + margin) / lambda;
                let price = (floor * scale * (1.0 + 0.35 * k as f64) + k as f64).ceil().max(1.0) as u32;
                let mine = my_gain[a.index()] - price as f64 * lambda;
                let theirs = price as f64 * lambda - q_loss[a.index()];
                if price > coins || my_gain[a.index()] * m.prm.buy_frac < price as f64 * lambda {
                    continue;
                }
                let terms = TradeTerms {
                    give: Assets::new(),
                    give_coins: price,
                    receive: single(a),
                    receive_coins: 0,
                };
                consider(mine - dq * theirs, terms, key, &mut best);
            }
            // Swap a spare token for one I want.
            if m.prm.swaps > 0.5 {
                for &a in &wants {
                    for &b in &spares {
                        if a == b {
                            continue;
                        }
                        let key = (qi, Some(a), Some(b));
                        let k = self.level(key);
                        if k >= 3 {
                            continue;
                        }
                        let their_net = q_gain[b.index()] - q_loss[a.index()];
                        let need = margin * (1.0 + k as f64) - their_net;
                        let give_coins = if need > 0.0 { ((need / lambda).ceil() as u32).min(coins) } else { 0 };
                        let c = give_coins as f64 * lambda;
                        let mine = my_gain[a.index()] - my_loss[b.index()] - c;
                        let theirs = their_net + c;
                        if mine < 0.5 {
                            continue;
                        }
                        let terms = TradeTerms {
                            give: single(b),
                            give_coins,
                            receive: single(a),
                            receive_coins: 0,
                        };
                        consider(mine - dq * theirs, terms, key, &mut best);
                    }
                }
            }
            // Sell a spare token for coins (pays only with players who are no threat).
            if m.prm.sells > 0.5 {
                for &b in &spares {
                    let key = (qi, None, Some(b));
                    let k = self.level(key);
                    if k >= 3 {
                        continue;
                    }
                    let pts = q_gain[b.index()] - margin * (1.0 + k as f64);
                    let price = (pts / lambda).floor().min(q.coins as f64);
                    if price < 1.0 {
                        continue;
                    }
                    let price = price as u32;
                    let mine = price as f64 * lambda - my_loss[b.index()];
                    let theirs = q_gain[b.index()] - price as f64 * lambda;
                    let terms = TradeTerms {
                        give: single(b),
                        give_coins: 0,
                        receive: Assets::new(),
                        receive_coins: price,
                    };
                    consider(mine - dq * theirs, terms, key, &mut best);
                }
            }
        }
        match best {
            Some((_, action, k)) => {
                self.proposals += 1;
                self.pending = Some(k);
                action
            }
            None => Action::EndTrading,
        }
    }

    /// Rejections of this kind of offer this turn.
    fn level(&self, key: TradeKey) -> usize {
        self.rejected.iter().filter(|k| **k == key).count()
    }
}

// ---- Closing ceremonies ------------------------------------------------------------

fn close(m: &Model, me: PlayerId, legal: &[Action]) -> Action {
    let g = m.g;
    let h0 = Hyp::of(g, me);
    let t = m.timing(me);
    let w = threat_weights(m, me);
    let donor = match g.phase() {
        Phase::Closing(c) => c.donation.map(|(_, d)| d),
        _ => None,
    };
    let donor_base = donor.map(|d| (d, m.timing(d), Hyp::of(g, d)));
    let donor_v0 = donor_base.as_ref().map(|(d, td, hd)| m.value(d.0, hd, td));
    let mut best: Option<(f64, &Action)> = None;
    for a in legal {
        let v = match a {
            Action::DiscardAsset { asset } => {
                let mut h = h0.clone();
                h.assets.remove(*asset, 1);
                m.value(me.0, &h, &t)
            }
            Action::RequestAsset { asset } => {
                let mut h = h0.clone();
                h.assets.add(*asset, 1);
                let mut v = m.value(me.0, &h, &t);
                if let (Some((d, td, hd)), Some(v0)) = (&donor_base, donor_v0) {
                    let mut h2 = hd.clone();
                    h2.assets.remove(*asset, 1);
                    let loss = (v0 - m.value(d.0, &h2, td)).max(0.0);
                    v += m.prm.deny * w[d.0] * loss;
                }
                v
            }
            _ => continue,
        };
        if best.is_none_or(|(b, _)| v > b + 1e-9) {
            best = Some((v, a));
        }
    }
    best.map(|(_, a)| a.clone()).unwrap_or_else(|| legal[0].clone())
}

impl Agent for ChameleonAgent {
    fn name(&self) -> String {
        "chameleon".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        if legal.is_empty() {
            return Action::EndTrading;
        }
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
        if game.history().len() < self.seen {
            // A new game with the same agent.
            self.scale.clear();
            self.trade_key = (0, usize::MAX);
        }
        self.seen = game.history().len();
        if self.reqs.len() != game.config().events.len() {
            self.reqs = build_reqs(game);
        }
        let prm = self.prm.clone();
        let reqs = std::mem::take(&mut self.reqs);
        let chosen = {
            let m = Model::new(game, &prm, &reqs);
            match game.phase() {
                Phase::Investing { remaining, .. } => invest(&m, me, legal, *remaining),
                Phase::Acquiring(_) => acquire(&m, me, legal),
                Phase::Trading { .. } => self.trade(&m, me, legal),
                Phase::Producing(pr) => produce::produce(&m, me, legal, pr),
                Phase::Closing(_) => close(&m, me, legal),
                Phase::GameOver => legal[0].clone(),
            }
        };
        self.reqs = reqs;
        let ok = match &chosen {
            Action::ProposeTrade { .. } | Action::CounterTrade { .. } => game.is_legal(&chosen),
            a => legal.contains(a),
        };
        if ok {
            chosen
        } else {
            self.pending = None;
            safe_fallback(legal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colosseum_engine::GameConfig;
    use colosseum_engine::agent::{GreedyAgent, RandomAgent};
    use std::sync::Arc;

    fn play(n: usize, seed: u64, mixed: bool) -> Game {
        let config = Arc::new(GameConfig::embedded());
        let mut game = Game::new(config, n, seed).unwrap();
        let mut agents: Vec<Box<dyn Agent>> = (0..n)
            .map(|i| -> Box<dyn Agent> {
                if !mixed || i % 3 == 0 {
                    Box::new(ChameleonAgent::new(seed + i as u64))
                } else if i % 3 == 1 {
                    Box::new(GreedyAgent::new(seed + i as u64))
                } else {
                    Box::new(RandomAgent::new(seed + i as u64))
                }
            })
            .collect();
        let mut steps = 0;
        while let Some(p) = game.current_player() {
            let legal = game.legal_actions();
            let a = agents[p.0].choose_action(&game, &legal);
            game.apply(a).expect("chameleon must choose legal actions");
            steps += 1;
            assert!(steps < 50_000, "game did not terminate");
        }
        game
    }

    #[test]
    fn plays_full_games_legally() {
        for n in 3..=5 {
            for seed in 0..8u64 {
                assert!(play(n, seed, false).is_over());
                assert!(play(n, seed + 100, true).is_over());
            }
        }
    }
}
