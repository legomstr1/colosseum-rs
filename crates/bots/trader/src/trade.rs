//! Pricing and building trade offers.

use crate::value::{StarSet, Weights, star_holder, stars_now, worth, worth_now};
use colosseum_engine::{Action, Asset, Assets, Game, PlayerId, TradeTerms};

/// How much a rival's gain counts against my own when judging a trade.
pub fn rho() -> f64 {
    crate::value::tune("rho", 0.1)
}

/// How a trade changes the worth of each side's position.
pub struct Eval {
    pub from: f64,
    pub to: f64,
}

/// Baseline worth of both sides, reused across many candidate terms.
pub struct Pair<'a> {
    pub game: &'a Game,
    pub w: &'a Weights,
    pub from: PlayerId,
    pub to: PlayerId,
    base_from: f64,
    base_to: f64,
    stars_from: StarSet,
    stars_to: StarSet,
}

impl<'a> Pair<'a> {
    pub fn new(game: &'a Game, w: &'a Weights, from: PlayerId, to: PlayerId) -> Self {
        Self {
            game,
            w,
            from,
            to,
            base_from: worth_now(game, w, from),
            base_to: worth_now(game, w, to),
            stars_from: stars_now(game, from),
            stars_to: stars_now(game, to),
        }
    }

    /// Worth change for `from` and `to` if `terms` (from `from`'s view) go
    /// through. Assumes the terms are affordable.
    pub fn eval(&self, terms: &TradeTerms) -> Eval {
        let (a, b) = (self.game.player(self.from), self.game.player(self.to));
        let mut af: Assets = a.assets.clone();
        let mut bf: Assets = b.assets.clone();
        af.remove_all(&terms.give);
        af.add_all(&terms.receive);
        bf.remove_all(&terms.receive);
        bf.add_all(&terms.give);
        let mut sf = self.stars_from;
        let mut st = self.stars_to;
        for &asset in &self.game.rules().stars.assets {
            if terms.give.count(asset) != 0 || terms.receive.count(asset) != 0 {
                let over = [(self.from, af.count(asset)), (self.to, bf.count(asset))];
                let h = star_holder(self.game, asset, &over);
                sf[asset.index()] = h == Some(self.from);
                st[asset.index()] = h == Some(self.to);
            }
        }
        let ca = (a.coins + terms.receive_coins).saturating_sub(terms.give_coins);
        let cb = (b.coins + terms.give_coins).saturating_sub(terms.receive_coins);
        Eval {
            from: worth(self.game, self.w, self.from, &af, ca, &sf, a.arena.len, None) - self.base_from,
            to: worth(self.game, self.w, self.to, &bf, cb, &st, b.arena.len, None) - self.base_to,
        }
    }
}

pub struct Candidate {
    pub terms: TradeTerms,
    pub mine: f64,
    pub theirs: f64,
}

fn single(a: Asset) -> Assets {
    let mut x = Assets::new();
    x.add(a, 1);
    x
}

/// Small bundles worth trying: singles, plus a pair or two of the best.
fn bundles(types: &[Asset], have: &Assets, with_empty: bool) -> Vec<Assets> {
    let mut out = Vec::new();
    if with_empty {
        out.push(Assets::new());
    }
    for &a in types {
        out.push(single(a));
    }
    for (i, &a) in types.iter().take(3).enumerate() {
        if have.count(a) >= 2 {
            let mut x = single(a);
            x.add(a, 1);
            out.push(x);
        }
        for &b in types.iter().take(3).skip(i + 1) {
            let mut x = single(a);
            x.add(b, 1);
            out.push(x);
        }
    }
    out
}

/// The offer from `me` to `other` that is best for me while still being worth
/// at least `their_min` to them (by my own reckoning). `skip` lists terms that
/// were already turned down. `counter` builds a counter-offer instead.
pub fn best_offer(
    game: &Game,
    w: &Weights,
    me: PlayerId,
    other: PlayerId,
    their_min: f64,
    skip: &[TradeTerms],
    counter: bool,
) -> Option<Candidate> {
    let pair = Pair::new(game, w, me, other);
    let (mp, op) = (game.player(me), game.player(other));
    let tradeable: Vec<Asset> = Asset::ALL.into_iter().filter(|a| a.is_tradeable()).collect();

    // What I'd like from them: tokens they hold that lift my position.
    let mut wants: Vec<(f64, Asset)> = tradeable
        .iter()
        .filter(|&&a| op.assets.count(a) > 0)
        .map(|&a| {
            let t = TradeTerms {
                receive: single(a),
                ..Default::default()
            };
            (pair.eval(&t).from, a)
        })
        .filter(|(g, _)| *g > 0.5)
        .collect();
    if wants.is_empty() {
        return None;
    }
    wants.sort_by(|a, b| b.0.total_cmp(&a.0));
    let want_types: Vec<Asset> = wants.iter().take(4).map(|(_, a)| *a).collect();
    let recv = bundles(&want_types, &op.assets, false);

    // What I can spare: tokens that cost me little and help them.
    let mut spares: Vec<(f64, Asset)> = tradeable
        .iter()
        .filter(|&&a| mp.assets.count(a) > 0)
        .map(|&a| {
            let t = TradeTerms {
                give: single(a),
                ..Default::default()
            };
            let e = pair.eval(&t);
            (e.to + e.from, a)
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();
    spares.sort_by(|a, b| b.0.total_cmp(&a.0));
    let give_types: Vec<Asset> = spares.iter().take(4).map(|(_, a)| *a).collect();
    let gives = bundles(&give_types, &mp.assets, true);

    let coin_w = w.coin.max(0.01);
    let mut best: Option<Candidate> = None;
    let mut score_best = f64::NEG_INFINITY;
    for r in &recv {
        for g in &gives {
            if !mp.assets.contains_all(g) || !op.assets.contains_all(r) {
                continue;
            }
            let mut terms = TradeTerms {
                give: g.clone(),
                receive: r.clone(),
                ..Default::default()
            };
            let e0 = pair.eval(&terms);
            // Settle the difference in coins so they just clear `their_min`.
            let c = ((their_min + rho() * e0.from - e0.to) / (coin_w * (1.0 + rho())))
                .ceil()
                .clamp(-12.0, 12.0) as i64;
            if c > 0 {
                terms.give_coins = (c as u32).min(mp.coins);
            } else if c < 0 {
                terms.receive_coins = ((-c) as u32).min(op.coins);
            }
            if terms.is_empty() || skip.contains(&terms) {
                continue;
            }
            let e = pair.eval(&terms);
            if e.from < crate::value::tune("minme", 0.7) || e.to - rho() * e.from < their_min - 0.2 {
                continue;
            }
            let s = e.from - crate::value::tune("objw", 0.4) * e.to.max(0.0);
            if s > score_best {
                let action = if counter {
                    Action::CounterTrade { terms: terms.clone() }
                } else {
                    Action::ProposeTrade {
                        to: other,
                        terms: terms.clone(),
                    }
                };
                if game.is_legal(&action) {
                    score_best = s;
                    best = Some(Candidate {
                        terms,
                        mine: e.from,
                        theirs: e.to,
                    });
                }
            }
        }
    }
    best
}
