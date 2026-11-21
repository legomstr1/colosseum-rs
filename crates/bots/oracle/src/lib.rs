//! Oracle: a cheap search bot.
//!
//! One fast valuation ([`model`]: the projected final-event score a holding
//! supports, plus a concave coin term) drives every decision, and shallow
//! exact search is spent where it pays most:
//! - producing: every line of dice moves is searched on plain arrays (no game
//!   clones). Before the final round the leaf values the event (coins, the
//!   "previous programs" bonus, the token lost at the closing), medals earned
//!   and the chance of taking the podium; in the final round it is my exact
//!   score after medal pulls, turned into a win probability against the
//!   players who still have to produce;
//! - auctions: a lot is worth my gain plus a share of what it gives the most
//!   threatening rival, minus what I could win elsewhere; as initiator I open
//!   the lot with the best expected surplus over the rivals' estimated
//!   ceilings;
//! - trading: buy needed tokens for coins (or swap spare ones) on a rising
//!   price ladder that remembers each partner's answers, and accept offers
//!   only when I gain clearly more than the proposer, weighted by how much
//!   of a threat they are;
//! - investing: the investment that raises the valuation most (tickets and
//!   the loge pay back every round, programs and arena squares only if they
//!   lift the final event).

mod model;
mod produce;
mod trade;

pub use model::Params;

use colosseum_engine::agent::Agent;
use colosseum_engine::{Acquiring, Action, Asset, Game, Phase, Side};
use model::{Ctx, Hyp, Tables};
use trade::TradeMemory;

pub struct OracleAgent {
    params: Params,
    tables: Option<Tables>,
    trade: TradeMemory,
}

impl OracleAgent {
    pub fn new(_seed: u64) -> Self {
        Self::with_params(Params::default())
    }

    pub fn with_params(params: Params) -> Self {
        Self {
            params,
            tables: None,
            trade: TradeMemory::default(),
        }
    }
}

/// Always-legal fallbacks, in order of preference.
const SAFE: [Action; 6] = [
    Action::PassBid,
    Action::PassInitiative,
    Action::EndInvesting,
    Action::EndTrading,
    Action::RejectTrade,
    Action::SkipEvent,
];

// ---- Investing --------------------------------------------------------------

/// Investments affordable now, with the holding each leads to.
fn investments(c: &Ctx, h0: &Hyp) -> Vec<(Action, Hyp)> {
    let g = c.g;
    let rules = g.rules();
    let me = g.player(c.me);
    let mut out = Vec::new();
    for &program in g.programs_for_sale() {
        if let Some(p) = c.t.prog(program)
            && h0.coins >= p.cost
        {
            let mut h = *h0;
            h.coins -= p.cost;
            h.add_prog(program);
            out.push((Action::BuyProgram { program }, h));
        }
    }
    if h0.arena < rules.arena.max_squares && h0.coins >= rules.arena.expansion_cost {
        let size = rules.board.size;
        let mut best: Option<(f64, Side)> = None;
        for side in [Side::Left, Side::Right] {
            let sq = me.arena.expansion_square(side, size);
            if g.arena_owner(sq).is_some() {
                continue;
            }
            let mut score = 0.0;
            for n in g.nobles() {
                let v = rules.noble_value(n.kind) as f64;
                // Nobles move clockwise; a noble a few squares behind the new
                // square is likely to land on it.
                let behind = (sq as i32 - n.position as i32).rem_euclid(size as i32) as f64;
                if behind == 0.0 {
                    score += v;
                } else if behind <= 6.0 {
                    score += v / (1.0 + behind);
                }
            }
            if rules.is_resting_place(sq) {
                score += 1.0;
            }
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, side));
            }
        }
        if let Some((_, side)) = best {
            let mut h = *h0;
            h.coins -= rules.arena.expansion_cost;
            h.arena += 1;
            out.push((Action::ExpandArena { side }, h));
        }
    }
    if g.supply().season_tickets > 0 && h0.coins >= rules.investing.season_ticket_cost {
        let mut h = *h0;
        h.coins -= rules.investing.season_ticket_cost;
        h.tickets += 1;
        out.push((Action::BuySeasonTicket, h));
    }
    if !h0.loge && g.supply().emperor_loges > 0 && h0.coins >= rules.investing.emperor_loge_cost {
        let mut h = *h0;
        h.coins -= rules.investing.emperor_loge_cost;
        h.loge = true;
        out.push((Action::BuildLoge, h));
    }
    out
}

fn invest(c: &Ctx, legal: &[Action], remaining: u32) -> Action {
    let me = c.me.0;
    let h0 = c.hyp(me);
    let rules = c.g.rules();
    if remaining > 0 {
        let tm0 = c.timing(remaining);
        let tm1 = c.timing(remaining - 1);
        let base = c.value(me, &h0, &tm0, None);
        let mut best: Option<(f64, Action)> = None;
        for (a, h) in investments(c, &h0) {
            let gain = c.value(me, &h, &tm1, None) - base;
            if legal.contains(&a) && best.as_ref().is_none_or(|b| gain > b.0) {
                best = Some((gain, a));
            }
        }
        if let Some((gain, a)) = best
            && gain > c.p.invest_min
        {
            return a;
        }
        return Action::EndInvesting;
    }
    // No investment left: buy another one if what it buys beats its price.
    let tm0 = c.timing(0);
    let base = c.value(me, &h0, &tm0, None);
    let mut extras: Vec<(Action, Hyp)> = Vec::new();
    if legal.contains(&Action::PlaySpecialEvent) {
        let mut h = h0;
        h.a[Asset::SpecialEvent.index()] -= 1;
        extras.push((Action::PlaySpecialEvent, h));
    }
    if legal.contains(&Action::SpendMedalsForInvestment) {
        let mut h = h0;
        h.medals -= rules.investing.extra_investment_medal_cost;
        extras.push((Action::SpendMedalsForInvestment, h));
    }
    let mut best: Option<(f64, Action)> = None;
    for (a, hx) in extras {
        let top = investments(c, &hx)
            .into_iter()
            .map(|(_, h)| c.value(me, &h, &tm0, None))
            .fold(f64::NEG_INFINITY, f64::max);
        let gain = top - base;
        if best.as_ref().is_none_or(|b| gain > b.0) {
            best = Some((gain, a));
        }
    }
    if let Some((gain, a)) = best
        && gain > c.p.invest_min
    {
        return a;
    }
    Action::EndInvesting
}

// ---- Acquiring --------------------------------------------------------------

/// Gain in points for `who` from winning `lot` for free.
fn lot_gain(c: &Ctx, who: usize, lot: &[Asset]) -> f64 {
    let mut tm = c.timing(0);
    tm.acq_left = (tm.acq_left - 1.0).max(0.0);
    let h0 = c.hyp(who);
    let mut h1 = h0;
    h1.add_tokens(lot, c.medal_supply);
    c.value(who, &h1, &tm, None) - c.value(who, &h0, &tm, None)
}

fn acquire(c: &Ctx, legal: &[Action], acq: &Acquiring) -> Action {
    let me = c.me.0;
    let markets = c.g.markets();
    let tm = c.timing(0);
    let coins = c.g.player(c.me).coins;
    let min = c.g.rules().acquiring.minimum_opening_bid;
    // (my gain, best rival gain, best rival ceiling in coins) for each lot.
    let vals: Vec<(f64, f64, f64)> = markets
        .iter()
        .map(|lot| {
            if lot.is_empty() {
                return (f64::NEG_INFINITY, 0.0, 0.0);
            }
            let mine = lot_gain(c, me, lot);
            let mut rival = 0.0f64;
            let mut rival_ceil = 0.0f64;
            for q in 0..c.n {
                if q != me && !acq.won.get(q).copied().unwrap_or(true) {
                    let gq = lot_gain(c, q, lot);
                    rival = rival.max(gq * c.threat[q]);
                    let cq = c.g.players()[q].coins as f64;
                    rival_ceil = rival_ceil.max(c.max_pay(gq, cq, &tm).floor());
                }
            }
            (mine, rival, rival_ceil)
        })
        .collect();
    let worth = |m: usize| vals[m].0 + c.p.deny * vals[m].1;
    match &acq.auction {
        None => {
            // Open the lot with the best expected surplus: my worth minus
            // what beating the keenest rival should cost.
            let mut best: Option<(f64, usize)> = None;
            let mut fallback: Option<(f64, usize)> = None;
            for m in 0..markets.len() {
                if markets[m].is_empty() || !legal.contains(&Action::StartAuction { market: m, bid: min }) {
                    continue;
                }
                let w = worth(m);
                let my_ceil = c.max_pay(c.p.bid_frac * w, coins as f64, &tm);
                if my_ceil < c.p.open_frac * min as f64 {
                    continue;
                }
                if fallback.is_none_or(|b| w > b.0) {
                    fallback = Some((w, m));
                }
                let price = (vals[m].2 + 1.0).max(min as f64);
                if price <= my_ceil {
                    let surplus = w - c.pay_cost(price, coins as f64, &tm);
                    if best.is_none_or(|b| surplus > b.0) {
                        best = Some((surplus, m));
                    }
                }
            }
            let pick = if c.p.smart_open > 0.0 { best.or(fallback) } else { fallback };
            match pick {
                Some((_, market)) => Action::StartAuction { market, bid: min },
                None => Action::PassInitiative,
            }
        }
        Some(au) => {
            let m = au.market;
            if m >= vals.len() {
                return Action::PassBid;
            }
            let alt = (0..markets.len())
                .filter(|&j| j != m && !markets[j].is_empty())
                .map(|j| vals[j].0 - c.pay_cost(min as f64, coins as f64, &tm))
                .fold(0.0f64, f64::max);
            let pts = worth(m) - c.p.alt_frac * alt;
            let ceil = (c.max_pay(c.p.bid_frac * pts, coins as f64, &tm).floor().max(0.0) as u32).min(coins);
            let bid = Action::Bid { amount: au.high_bid + 1 };
            if au.high_bid < ceil && legal.contains(&bid) {
                bid
            } else {
                Action::PassBid
            }
        }
    }
}

// ---- Closing ----------------------------------------------------------------

fn close(c: &Ctx, legal: &[Action]) -> Action {
    let me = c.me.0;
    let tm = c.timing(0);
    let h0 = c.hyp(me);
    let mut best: Option<(f64, &Action)> = None;
    for a in legal {
        let v = match a {
            Action::DiscardAsset { asset } => {
                let mut h = h0;
                if h.a[asset.index()] == 0 {
                    continue;
                }
                h.a[asset.index()] -= 1;
                c.value(me, &h, &tm, None)
            }
            Action::RequestAsset { asset } => {
                let mut h = h0;
                h.a[asset.index()] += 1;
                let mine = c.value(me, &h, &tm, None);
                // Taking it also hurts the donor.
                let donor = match c.g.phase() {
                    Phase::Closing(cl) => cl.donation.map(|(_, d)| d.0),
                    _ => None,
                };
                let hurt = donor.map_or(0.0, |d| {
                    let q0 = c.hyp(d);
                    let mut q1 = q0;
                    if q1.a[asset.index()] > 0 {
                        q1.a[asset.index()] -= 1;
                    }
                    c.value(d, &q0, &tm, None) - c.value(d, &q1, &tm, None)
                });
                mine + c.p.deny * hurt * donor.map_or(1.0, |d| c.threat[d])
            }
            _ => continue,
        };
        if best.is_none_or(|b| v > b.0) {
            best = Some((v, a));
        }
    }
    best.map(|b| b.1.clone()).or_else(|| legal.first().cloned()).unwrap_or(Action::EndTrading)
}

impl OracleAgent {
    fn decide(&mut self, game: &Game, legal: &[Action]) -> Option<Action> {
        let me = game.current_player()?;
        if self.tables.is_none() {
            self.tables = Some(Tables::new(game));
        }
        let tables = self.tables.as_ref()?;
        let c = Ctx::new(game, tables, &self.params, me);
        let c = match game.phase() {
            Phase::Acquiring(_) | Phase::Trading { .. } | Phase::Closing(_) => c.with_threats(),
            _ => c,
        };
        Some(match game.phase() {
            Phase::Investing { remaining, .. } => invest(&c, legal, *remaining),
            Phase::Acquiring(acq) => acquire(&c, legal, acq),
            Phase::Trading { offer: None, .. } => trade::propose(&c, &mut self.trade),
            Phase::Trading { offer: Some(o), .. } => {
                // A counter to my offer: its answer is not news about my price.
                self.trade.forget_pending();
                trade::respond(&c, legal, o)
            }
            Phase::Producing(pr) => produce::produce(&c, legal, pr),
            Phase::Closing(_) => close(&c, legal),
            Phase::GameOver => return None,
        })
    }
}

impl Agent for OracleAgent {
    fn name(&self) -> String {
        "oracle".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        if let Some(a) = self.decide(game, legal) {
            let ok = match &a {
                Action::ProposeTrade { .. } | Action::CounterTrade { .. } => game.is_legal(&a),
                _ => legal.contains(&a),
            };
            if ok {
                return a;
            }
        }
        SAFE.iter()
            .find(|a| legal.contains(a))
            .cloned()
            .or_else(|| legal.first().cloned())
            .unwrap_or(Action::EndTrading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colosseum_engine::GameConfig;
    use std::sync::Arc;

    fn play(n: usize, seed: u64, all_oracle: bool) -> Game {
        let config = Arc::new(GameConfig::embedded());
        let mut game = Game::new(config, n, seed).unwrap();
        let mut agents: Vec<Box<dyn Agent>> = (0..n)
            .map(|i| -> Box<dyn Agent> {
                if all_oracle || i % 2 == 0 {
                    Box::new(OracleAgent::new(seed + i as u64))
                } else {
                    Box::new(colosseum_engine::agent::GreedyAgent::new(seed + i as u64))
                }
            })
            .collect();
        let mut steps = 0;
        while let Some(p) = game.current_player() {
            let legal = game.legal_actions();
            let a = agents[p.0].choose_action(&game, &legal);
            assert!(game.is_legal(&a), "illegal {a:?}");
            game.apply(a).expect("legal");
            steps += 1;
            assert!(steps < 50_000, "game did not terminate");
        }
        game
    }

    #[test]
    fn plays_full_games_legally() {
        for n in 3..=5 {
            for seed in 0..4u64 {
                assert!(play(n, seed, true).is_over());
                assert!(play(n, seed + 100, false).is_over());
            }
        }
    }
}
