//! Trading: buy tokens I need from opponents with coins (raising the price
//! after each refusal), and accept offers only when they help me clearly
//! more than they help the proposer.

use crate::model::{Ctx, Hyp, MAXP, NA};
use colosseum_engine::{Action, Asset, Assets, Offer, Phase, PlayerId, TradeTerms};

/// Coin prices tried for a token, in rising order.
const LADDER: [u32; 12] = [1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 19, 24];
/// Coins added to a token swap, in rising order.
const SWAP_LADDER: [u32; 5] = [0, 1, 2, 4, 7];

/// Negotiation memory: per trading turn, the price level reached for each
/// (partner, asset); over the whole game, how each partner answers.
#[derive(Default)]
pub struct TradeMemory {
    key: (u32, usize),
    offers: u32,
    /// Ladder level reached for (partner, asset) this turn.
    level: Vec<(usize, usize, usize)>,
    /// My last offer: (partner, asset, price).
    pending: Option<(usize, usize, u32)>,
    /// Offers made to and accepted by each partner, this game.
    tries: [u32; MAXP],
    accepts: [u32; MAXP],
    /// Offers to each partner this turn.
    turn_tries: [u32; MAXP],
    /// Lowest price each partner has accepted.
    floor: [u32; MAXP],
    use_floor: bool,
}

impl TradeMemory {
    /// Ladder level for (partner, key): keys below `NA` are coin offers for
    /// one asset, larger keys are swaps.
    fn level_of(&self, q: usize, a: usize) -> usize {
        self.level
            .iter()
            .find(|(x, y, _)| *x == q && *y == a)
            .map_or_else(|| if a < NA { self.start_level(q) } else { 0 }, |e| e.2)
    }

    /// Where to start the ladder with a partner: at the cheapest price they
    /// have taken before.
    fn start_level(&self, q: usize) -> usize {
        let f = self.floor[q];
        if f == 0 || !self.use_floor {
            return 0;
        }
        LADDER.iter().position(|&p| p >= f).unwrap_or(0)
    }

    pub fn forget_pending(&mut self) {
        self.pending = None;
    }

    fn bump(&mut self, q: usize, a: usize, to: usize) {
        if let Some(e) = self.level.iter_mut().find(|(x, y, _)| *x == q && *y == a) {
            e.2 = e.2.max(to);
        } else {
            self.level.push((q, a, to));
        }
    }

    /// Offers still worth making to `q` this turn: partners who never accept
    /// get only an occasional probe.
    fn budget(&self, q: usize, probe: u32) -> u32 {
        if self.accepts[q] == 0 && self.tries[q] >= probe {
            1
        } else {
            u32::MAX
        }
    }
}

fn apply_terms(h: &mut Hyp, gain: &Assets, gain_coins: u32, lose: &Assets, lose_coins: u32) -> bool {
    for (a, n) in lose.iter() {
        if h.a[a.index()] < n {
            return false;
        }
        h.a[a.index()] -= n;
    }
    for (a, n) in gain.iter() {
        h.a[a.index()] += n;
    }
    if h.coins + gain_coins < lose_coins {
        return false;
    }
    h.coins = h.coins + gain_coins - lose_coins;
    true
}

/// Weight of a partner's gain against mine: helping one rival matters less
/// the more rivals there are.
fn trade_k(c: &Ctx, q: usize) -> f64 {
    c.p.trade_k * 2.0 / (c.n.max(2) - 1) as f64 * c.threat.get(q).copied().unwrap_or(1.0)
}

/// My answer to an offer made to me.
pub fn respond(c: &Ctx, legal: &[Action], o: &Offer) -> Action {
    if !legal.contains(&Action::AcceptTrade) {
        return Action::RejectTrade;
    }
    let me = c.me.0;
    let them = o.from.0;
    let tm = c.timing(0);
    let t = &o.terms;
    let h0 = c.hyp(me);
    let mut h1 = h0;
    // I receive `give`, hand over `receive`.
    if !apply_terms(&mut h1, &t.give, t.give_coins, &t.receive, t.receive_coins) {
        return Action::RejectTrade;
    }
    let q0 = c.hyp(them);
    let mut q1 = q0;
    if !apply_terms(&mut q1, &t.receive, t.receive_coins, &t.give, t.give_coins) {
        return Action::RejectTrade;
    }
    let mine = c.value(me, &h1, &tm, Some((them, &q1.a))) - c.value(me, &h0, &tm, None);
    let theirs = c.value(them, &q1, &tm, Some((me, &h1.a))) - c.value(them, &q0, &tm, None);
    if mine - trade_k(c, them) * theirs.max(0.0) > c.p.accept_margin && mine > c.p.accept_margin {
        Action::AcceptTrade
    } else {
        Action::RejectTrade
    }
}

pub fn propose(c: &Ctx, mem: &mut TradeMemory) -> Action {
    let Phase::Trading { turn, offers, .. } = c.g.phase() else {
        return Action::EndTrading;
    };
    let key = (c.round, *turn);
    mem.use_floor = c.p.use_floor > 0.0;
    if mem.key != key {
        mem.key = key;
        mem.offers = 0;
        mem.level.clear();
        mem.pending = None;
        mem.turn_tries = [0; MAXP];
    }
    // Learn from the answer to my last offer.
    if let Some((q, a, price)) = mem.pending.take() {
        if matches!(c.g.history().last(), Some(Action::AcceptTrade)) {
            mem.accepts[q] += 1;
            if a < NA && (mem.floor[q] == 0 || price < mem.floor[q]) {
                mem.floor[q] = price;
            }
        } else {
            let lvl = mem.level_of(q, a);
            mem.bump(q, a, lvl + 1);
        }
    }
    if mem.offers >= c.p.max_offers {
        return Action::EndTrading;
    }
    let me = c.me.0;
    let tm = c.timing(0);
    let h0 = c.hyp(me);
    let base = c.value(me, &h0, &tm, None);
    let cap = c.g.rules().trading.max_offers_per_partner;
    let lambda = tm.lambda.max(1e-3);
    // My loss from giving away one of each token I hold.
    let mut my_loss = [f64::INFINITY; NA];
    if c.p.swaps > 0.0 {
        for (b, loss) in my_loss.iter_mut().enumerate() {
            if h0.a[b] > 0 && Asset::ALL[b].is_tradeable() {
                let mut h1 = h0;
                h1.a[b] -= 1;
                *loss = base - c.value(me, &h1, &tm, None);
            }
        }
    }
    // (surplus, partner, ladder key, coins, asset given or NA).
    let mut best: Option<(f64, usize, usize, u32, usize)> = None;
    for q in 0..c.n {
        if q >= MAXP || q == me || offers.get(q).copied().unwrap_or(cap) >= cap {
            continue;
        }
        if mem.turn_tries[q] >= mem.budget(q, c.p.probe_tries) {
            continue;
        }
        let k = trade_k(c, q);
        let q0 = c.hyp(q);
        let qbase = c.value(q, &q0, &tm, None);
        // Tokens I want from q: (my gain, their loss, asset).
        let mut wants: Vec<(f64, f64, usize)> = Vec::new();
        for a in 0..NA {
            if q0.a[a] == 0 || !Asset::ALL[a].is_tradeable() {
                continue;
            }
            let mut h1 = h0;
            h1.a[a] += 1;
            let mut q1 = q0;
            q1.a[a] -= 1;
            let gain = c.value(me, &h1, &tm, Some((q, &q1.a))) - base;
            if gain < c.p.buy_min {
                continue;
            }
            let loss = (qbase - c.value(q, &q1, &tm, Some((me, &h1.a)))).max(0.0);
            wants.push((gain, loss, a));
            // Paying p coins: my net gain - cost(p), theirs lambda*p - loss.
            let rel = gain + k * loss;
            let lvl = mem.level_of(q, a);
            let Some(&price) = LADDER.get(lvl) else { continue };
            if price > h0.coins {
                continue;
            }
            let cost = c.pay_cost(price as f64, h0.coins as f64, &tm) + k * lambda * price as f64;
            if cost > c.p.pay_frac * rel {
                continue;
            }
            let surplus = rel - cost;
            if best.is_none_or(|b| surplus > b.0) {
                best = Some((surplus, q, a, price, NA));
            }
        }
        if c.p.swaps <= 0.0 || wants.is_empty() {
            continue;
        }
        wants.sort_by(|x, y| y.0.total_cmp(&x.0));
        wants.truncate(3);
        // What each of my spare tokens is worth to q.
        let mut their_gain = [0.0f64; NA];
        for b in 0..NA {
            if my_loss[b].is_finite() {
                let mut q1 = q0;
                q1.a[b] += 1;
                their_gain[b] = c.value(q, &q1, &tm, None) - qbase;
            }
        }
        for &(gain, loss, a) in &wants {
            for b in 0..NA {
                if b == a || !my_loss[b].is_finite() || their_gain[b] < c.p.swap_min {
                    continue;
                }
                let mine = gain - my_loss[b];
                if mine < c.p.buy_min {
                    continue;
                }
                let key = NA + a * NA + b;
                let lvl = mem.level_of(q, key);
                let Some(&price) = SWAP_LADDER.get(lvl) else { continue };
                if price > h0.coins {
                    continue;
                }
                let rel = mine + k * (loss - their_gain[b]);
                let cost = c.pay_cost(price as f64, h0.coins as f64, &tm) + k * lambda * price as f64;
                if cost > c.p.pay_frac * rel {
                    continue;
                }
                let surplus = c.p.swaps * (rel - cost);
                if best.is_none_or(|x| surplus > x.0) {
                    best = Some((surplus, q, key, price, b));
                }
            }
        }
    }
    if let Some((_, q, key, price, b)) = best {
        let a = if key < NA { key } else { (key - NA) / NA };
        let mut receive = Assets::new();
        receive.add(Asset::ALL[a], 1);
        let mut give = Assets::new();
        if b < NA {
            give.add(Asset::ALL[b], 1);
        }
        let terms = TradeTerms {
            give,
            give_coins: price,
            receive,
            receive_coins: 0,
        };
        let action = Action::ProposeTrade { to: PlayerId(q), terms };
        if c.g.is_legal(&action) {
            mem.offers += 1;
            mem.tries[q] += 1;
            mem.turn_tries[q] += 1;
            mem.pending = Some((q, key, price));
            return action;
        }
    }
    Action::EndTrading
}
