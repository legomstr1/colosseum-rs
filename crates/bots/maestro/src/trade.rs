//! Trading: buy needed tokens for coins on a rising price ladder, offer
//! token swaps, and accept offers only when my gain clearly beats the
//! partner's. Valuations are cached for the turn and refreshed after any
//! trade; partners that never accept are offered less.

use crate::MaestroAgent;
use crate::model::{Ctx, Hold, MAXP};
use crate::tables::NA;
use colosseum_engine::{Action, Asset, Assets, Game, Offer, Phase, PlayerId, TradeTerms};

const LADDER: [u32; 9] = [1, 2, 3, 5, 7, 10, 14, 19, 25];

/// Valuations reused across one trading turn while holdings don't change.
#[derive(Clone)]
struct Cache {
    stamp: u64,
    /// My gain from one more token of each type.
    want: [f64; NA],
    /// What giving one token of each type away costs me.
    spare: [f64; NA],
    /// Each partner's loss from giving up / gain from receiving one token.
    qloss: [[f64; NA]; MAXP],
    qgain: [[f64; NA]; MAXP],
}

/// Negotiation memory.
#[derive(Default)]
pub(crate) struct TradeMem {
    key: (u32, usize),
    /// Price ladder level reached this turn per `(partner, asset)`.
    level: Vec<(u8, u8, u8)>,
    /// Swaps already offered this turn: `(partner, wanted, given)`.
    swapped: Vec<(u8, u8, u8)>,
    /// My last offer: `(partner, asset, ladder level if a coin buy)`.
    pub pending: Option<(u8, u8, Option<u8>)>,
    offers: u32,
    per_partner: [u32; MAXP],
    cache: Option<Cache>,
    /// Across the game, per partner: offers made, offers accepted, and the
    /// ladder level to start at.
    sent: [u32; MAXP],
    taken: [u32; MAXP],
    start: [u8; MAXP],
}

fn one(a: usize) -> Assets {
    let mut x = Assets::new();
    x.add(Asset::ALL[a], 1);
    x
}

/// Cheap fingerprint of every player's coins and tokens.
fn stamp(g: &Game) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in g.players() {
        h = (h ^ p.coins as u64).wrapping_mul(0x100_0000_01b3);
        for (a, n) in p.assets.iter() {
            h = (h ^ ((a.index() as u64) << 8 | n as u64)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// Worth change for `from` and `to` if `terms` (from `from`'s view) go through.
pub fn trade_eval(c: &Ctx, from: usize, to: usize, terms: &TradeTerms) -> (f64, f64) {
    let pl = c.g.players();
    let hf0 = Hold::of(&pl[from]);
    let ht0 = Hold::of(&pl[to]);
    let mut hf = hf0;
    let mut ht = ht0;
    for (a, n) in terms.give.iter() {
        let i = a.index();
        hf.a[i] = hf.a[i].saturating_sub(n as u8);
        ht.a[i] = ht.a[i].saturating_add(n as u8);
    }
    for (a, n) in terms.receive.iter() {
        let i = a.index();
        ht.a[i] = ht.a[i].saturating_sub(n as u8);
        hf.a[i] = hf.a[i].saturating_add(n as u8);
    }
    hf.coins += terms.receive_coins as i32 - terms.give_coins as i32;
    ht.coins += terms.give_coins as i32 - terms.receive_coins as i32;
    let mut cc = c.clone();
    cc.counts[from] = hf.a;
    cc.counts[to] = ht.a;
    (cc.value(&hf) - c.value(&hf0), cc.value(&ht) - c.value(&ht0))
}

fn build_cache(c: &Ctx, me: usize, st: u64) -> Cache {
    let pl = c.g.players();
    let mut want = [0.0; NA];
    let mut spare = [f64::INFINITY; NA];
    let mut qloss = [[f64::INFINITY; NA]; MAXP];
    let mut qgain = [[0.0; NA]; MAXP];
    let h0 = Hold::of(&pl[me]);
    let tm = c.timing(me);
    let base = c.value_t(&h0, &tm);
    for a in 0..NA {
        if !c.t.tradeable[a] {
            continue;
        }
        if (0..c.n).any(|q| q != me && c.counts[q][a] > 0) {
            let mut h = h0;
            h.a[a] += 1;
            want[a] = c.value_t(&h, &tm) - base;
        }
        if h0.a[a] > 0 {
            let mut h = h0;
            h.a[a] -= 1;
            spare[a] = base - c.value_t(&h, &tm);
        }
    }
    for q in (0..c.n).filter(|&q| q != me) {
        let hq = Hold::of(&pl[q]);
        let tq = c.timing(q);
        let bq = c.value_t(&hq, &tq);
        for a in 0..NA {
            if !c.t.tradeable[a] {
                continue;
            }
            if hq.a[a] > 0 && want[a] > 0.0 {
                let mut h = hq;
                h.a[a] -= 1;
                qloss[q][a] = bq - c.value_t(&h, &tq);
            }
            if h0.a[a] > 0 {
                let mut h = hq;
                h.a[a] += 1;
                qgain[q][a] = c.value_t(&h, &tq) - bq;
            }
        }
    }
    Cache {
        stamp: st,
        want,
        spare,
        qloss,
        qgain,
    }
}

impl MaestroAgent {
    fn sync_mem(&mut self, c: &Ctx) {
        let turn = match c.g.phase() {
            Phase::Trading { turn, .. } => *turn,
            _ => usize::MAX,
        };
        let key = (c.round, turn);
        if self.mem.key != key {
            let m = &mut self.mem;
            m.key = key;
            m.level.clear();
            m.swapped.clear();
            m.pending = None;
            m.offers = 0;
            m.per_partner = [0; MAXP];
            m.cache = None;
        }
    }

    /// Learns from the answer to my last offer.
    fn settle_pending(&mut self, accepted: bool) {
        let Some((q, a, level)) = self.mem.pending.take() else { return };
        let m = &mut self.mem;
        let qi = (q as usize).min(MAXP - 1);
        if accepted {
            m.taken[qi] += 1;
            if let Some(l) = level {
                m.start[qi] = l.saturating_sub(1);
            }
        } else if let Some(l) = level {
            if let Some(e) = m.level.iter_mut().find(|e| e.0 == q && e.1 == a) {
                e.2 = e.2.max(l + 1);
            } else {
                m.level.push((q, a, l + 1));
            }
        }
    }

    /// Offers allowed to `q` this turn, given how often they accept.
    fn partner_cap(&self, q: usize, learn: bool) -> u32 {
        let (sent, taken) = (self.mem.sent[q], self.mem.taken[q]);
        if !learn {
            return u32::MAX;
        }
        if sent >= 8 && taken * 12 < sent {
            1
        } else if sent >= 4 && taken * 5 < sent {
            3
        } else {
            u32::MAX
        }
    }

    pub(crate) fn propose(&mut self, c: &Ctx, me: usize) -> Action {
        self.sync_mem(c);
        let accepted = matches!(c.g.history().last(), Some(Action::AcceptTrade));
        self.settle_pending(accepted);
        let p = c.p;
        if self.mem.offers as f64 >= p.max_offers {
            return Action::EndTrading;
        }
        let g = c.g;
        let rule_cap = g.rules().trading.max_offers_per_partner;
        let made = match g.phase() {
            Phase::Trading { offers, .. } => offers,
            _ => return Action::EndTrading,
        };
        let st = stamp(g);
        if self.mem.cache.as_ref().is_none_or(|k| k.stamp != st) {
            self.mem.cache = Some(build_cache(c, me, st));
        }
        let k = self.mem.cache.as_ref().expect("built above");
        let lam = c.lam;
        let coins = g.players()[me].coins;

        // (score, partner, wanted, given or coin level)
        let mut best: Option<(f64, usize, usize, Result<u8, usize>)> = None;
        for q in (0..c.n).filter(|&q| q != me) {
            if made.get(q).copied().unwrap_or(rule_cap) >= rule_cap
                || self.mem.per_partner[q] >= self.partner_cap(q, p.learn == 1.0 || p.learn == 3.0)
            {
                continue;
            }
            for a in 0..NA {
                if k.want[a] < p.buy_min || c.counts[q][a] == 0 {
                    continue;
                }
                let level = self
                    .mem
                    .level
                    .iter()
                    .find(|e| e.0 as usize == q && e.1 as usize == a)
                    .map_or(if p.learn >= 2.0 { self.mem.start[q] } else { 0 }, |e| e.2);
                if let Some(&price) = LADDER.get(level as usize)
                    && price <= coins
                {
                    let dq = price as f64 * lam - k.qloss[q][a];
                    let s = k.want[a] - lam * price as f64 - p.rho * dq.max(0.0);
                    if s > p.trade_min && best.is_none_or(|b| s > b.0) {
                        best = Some((s, q, a, Ok(level)));
                    }
                }
                if p.swaps > 0.0 {
                    for b in 0..NA {
                        if b == a || !k.spare[b].is_finite() || k.spare[b] >= 1.0 {
                            continue;
                        }
                        if self.mem.swapped.contains(&(q as u8, a as u8, b as u8)) {
                            continue;
                        }
                        let dq = k.qgain[q][b] - k.qloss[q][a];
                        if dq < 0.2 {
                            continue;
                        }
                        let s = k.want[a] - k.spare[b] - p.rho * dq;
                        if s > p.trade_min && best.is_none_or(|bb| s > bb.0) {
                            best = Some((s, q, a, Err(b)));
                        }
                    }
                }
            }
        }
        let Some((_, q, a, kind)) = best else {
            return Action::EndTrading;
        };
        let terms = match kind {
            Ok(level) => TradeTerms {
                give: Assets::new(),
                give_coins: LADDER[level as usize],
                receive: one(a),
                receive_coins: 0,
            },
            Err(b) => TradeTerms {
                give: one(b),
                give_coins: 0,
                receive: one(a),
                receive_coins: 0,
            },
        };
        let action = Action::ProposeTrade { to: PlayerId(q), terms };
        if !g.is_legal(&action) {
            return Action::EndTrading;
        }
        let m = &mut self.mem;
        m.pending = Some((q as u8, a as u8, kind.ok()));
        if let Err(b) = kind {
            m.swapped.push((q as u8, a as u8, b as u8));
        }
        m.offers += 1;
        m.per_partner[q] += 1;
        m.sent[q] += 1;
        action
    }

    pub(crate) fn respond(&mut self, c: &Ctx, me: usize, legal: &[Action], offer: &Offer) -> Action {
        // A counter-offer to my offer means my terms were turned down.
        if self.mem.pending.is_some() {
            self.sync_mem(c);
            self.settle_pending(false);
        }
        if !legal.contains(&Action::AcceptTrade) {
            return Action::RejectTrade;
        }
        let (dthem, dme) = trade_eval(c, offer.from.0, me, &offer.terms);
        if dme - c.p.rho * dthem.max(0.0) > c.p.acc_min {
            Action::AcceptTrade
        } else {
            Action::RejectTrade
        }
    }
}
