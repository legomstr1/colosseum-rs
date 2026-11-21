//! One decision function per phase.

use crate::PlannerAgent;
use crate::model::{Hyp, Model, Timing};
use colosseum_engine::{Acquiring, Action, Asset, Assets, DieFace, Offer, Producing, Side, TradeTerms};

const EPS: f64 = 0.05;

// ---- Investing -------------------------------------------------------------

/// Every investment the player could pay for, with the holding it leads to.
fn investments(m: &Model, h0: &Hyp) -> Vec<(Action, Hyp)> {
    let g = m.game;
    let rules = g.rules();
    let me = g.player(m.me);
    let mut out = Vec::new();
    for &program in g.programs_for_sale() {
        let cost = g.config().event(program).map_or(u32::MAX, |e| e.cost);
        if h0.coins >= cost {
            let mut h = h0.clone();
            h.coins -= cost;
            h.owned.push((program, false));
            out.push((Action::BuyProgram { program }, h));
        }
    }
    if h0.arena_len < rules.arena.max_squares && h0.coins >= rules.arena.expansion_cost {
        let mut best: Option<(f64, Side)> = None;
        for side in [Side::Left, Side::Right] {
            let sq = me.arena.expansion_square(side, rules.board.size);
            if g.arena_owner(sq).is_some() {
                continue;
            }
            // Prefer the side whose new square holds, or is about to meet, a noble.
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

/// The best investment and how much it improves on `base`, when the player
/// will have `slots_after` investments left after making it.
fn best_investment(m: &Model, h0: &Hyp, t: &Timing, base: f64) -> Option<(Action, f64)> {
    let mut after = *t;
    after.slots = t.slots.saturating_sub(1);
    investments(m, h0)
        .into_iter()
        .map(|(a, h)| {
            let v = m.value(&h, &after) - base;
            (a, v)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

pub fn invest(m: &Model, legal: &[Action], remaining: u32) -> Option<Action> {
    let h0 = Hyp::of(m.game, m.me);
    let t = m.timing_now(true, remaining);
    let base = m.value(&h0, &t);
    if remaining > 0
        && let Some((a, gain)) = best_investment(m, &h0, &t, base)
        && gain > EPS
        && legal.contains(&a)
    {
        return Some(a);
    }
    if remaining == 0 {
        let rules = m.game.rules();
        // An extra investment is worth it if the best purchase beats what the
        // extra investment costs.
        if legal.contains(&Action::PlaySpecialEvent) {
            let mut t2 = t;
            t2.slots += rules.investing.special_event_extra_investments;
            if let Some((_, gain)) = best_investment(m, &h0, &t2, base)
                && gain > m.params.medal_value
            {
                return Some(Action::PlaySpecialEvent);
            }
        }
        if legal.contains(&Action::SpendMedalsForInvestment) {
            let mut h = h0.clone();
            h.medals -= rules.investing.extra_investment_medal_cost;
            let mut t2 = t;
            t2.slots += 1;
            if let Some((_, gain)) = best_investment(m, &h, &t2, base)
                && gain > EPS
            {
                return Some(Action::SpendMedalsForInvestment);
            }
        }
    }
    if legal.contains(&Action::ExchangeSpecialEventForMedal) {
        return Some(Action::ExchangeSpecialEventForMedal);
    }
    Some(Action::EndInvesting)
}

// ---- Acquiring ---------------------------------------------------------------

fn add_tokens(m: &Model, h: &mut Hyp, tokens: &[Asset]) {
    let mut medal_supply = m.game.supply().medals;
    for &tk in tokens {
        match tk {
            Asset::Emperor | Asset::SpecialEvent => {
                if medal_supply > 0 {
                    medal_supply -= 1;
                    h.medals += 1;
                }
            }
            _ => h.assets.add(tk, 1),
        }
    }
}

/// What a batch of tokens is worth, in points.
fn batch_value(m: &Model, tokens: &[Asset]) -> f64 {
    let h0 = Hyp::of(m.game, m.me);
    let mut t = m.timing_now(false, 0);
    t.acq_left *= m.params.kappa;
    let mut h1 = h0.clone();
    add_tokens(m, &mut h1, tokens);
    m.value(&h1, &t) - m.value(&h0, &t)
}

pub fn acquire(m: &Model, legal: &[Action], acq: &Acquiring) -> Option<Action> {
    let g = m.game;
    let lambda = m.lambda();
    let coins = g.player(m.me).coins;
    match &acq.auction {
        None => {
            let min = g.rules().acquiring.minimum_opening_bid;
            let mut best: Option<(f64, usize)> = None;
            for (i, batch) in g.markets().iter().enumerate() {
                if batch.is_empty() || !legal.contains(&Action::StartAuction { market: i, bid: min }) {
                    continue;
                }
                let v = batch_value(m, batch);
                if best.is_none_or(|(b, _)| v > b) {
                    best = Some((v, i));
                }
            }
            match best {
                Some((v, market)) if v * m.params.pay_frac / lambda >= min as f64 => {
                    Some(Action::StartAuction { market, bid: min })
                }
                _ => Some(Action::PassInitiative),
            }
        }
        Some(a) => {
            let v = batch_value(m, &g.markets()[a.market]);
            let cap = ((v * m.params.pay_frac / lambda).floor().max(0.0) as u32).min(coins);
            if a.high_bid < cap {
                Some(Action::Bid { amount: a.high_bid + 1 })
            } else {
                Some(Action::PassBid)
            }
        }
    }
}

// ---- Trading -------------------------------------------------------------------

pub fn respond(m: &Model, legal: &[Action], offer: &Offer) -> Action {
    if !legal.contains(&Action::AcceptTrade) {
        return Action::RejectTrade;
    }
    let h0 = Hyp::of(m.game, m.me);
    let t = m.timing_now(false, 0);
    let mut h1 = h0.clone();
    let terms: &TradeTerms = &offer.terms;
    if !h1.assets.remove_all(&terms.receive) {
        return Action::RejectTrade;
    }
    h1.assets.add_all(&terms.give);
    h1.coins = h1.coins + terms.give_coins - terms.receive_coins.min(h1.coins + terms.give_coins);
    if m.value(&h1, &t) - m.value(&h0, &t) > 0.3 {
        Action::AcceptTrade
    } else {
        Action::RejectTrade
    }
}

const LADDER: [u32; 12] = [1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 20, 25];

impl PlannerAgent {
    /// Tries to buy tokens the plan needs, opening low and raising each time
    /// an offer is turned down.
    pub(crate) fn propose(&mut self, m: &Model, _legal: &[Action]) -> Option<Action> {
        let g = m.game;
        let round = g.round();
        let h0 = Hyp::of(g, m.me);
        let t = m.timing_now(false, 0);
        let base = m.value(&h0, &t);
        let lambda = m.lambda();
        let mut best: Option<(f64, Action, usize, Asset, u32)> = None;
        for q in g.players() {
            if q.id == m.me {
                continue;
            }
            for (asset, _) in q.assets.iter() {
                if !asset.is_tradeable() {
                    continue;
                }
                let mut h1 = h0.clone();
                h1.assets.add(asset, 1);
                let gain = m.value(&h1, &t) - base;
                if gain < 1.0 {
                    continue;
                }
                let max_price = ((gain * 0.6 / lambda).floor() as u32).min(h0.coins);
                let last = self
                    .tried
                    .iter()
                    .filter(|(r, p, a, _)| *r == round && *p == q.id.0 && *a == asset)
                    .map(|x| x.3)
                    .max();
                let Some(price) = LADDER
                    .iter()
                    .copied()
                    .find(|&p| last.is_none_or(|l| p > l) && p <= max_price)
                else {
                    continue;
                };
                let terms = TradeTerms {
                    give: Assets::new(),
                    give_coins: price,
                    receive: [asset].into_iter().collect(),
                    receive_coins: 0,
                };
                let action = Action::ProposeTrade { to: q.id, terms };
                if g.is_legal(&action) && best.as_ref().is_none_or(|b| gain > b.0) {
                    best = Some((gain, action, q.id.0, asset, price));
                }
            }
        }
        match best {
            Some((_, action, q, asset, price)) => {
                self.tried.retain(|(r, ..)| *r + 1 >= round);
                self.tried.push((round, q, asset, price));
                Some(action)
            }
            None => Some(Action::EndTrading),
        }
    }
}

// ---- Producing -----------------------------------------------------------------

/// What each owned program would score right now, without nobles.
struct Option_ {
    program: u8,
    pre: f64,
    adj: f64,
}

fn options(m: &Model, final_round: bool) -> Vec<Option_> {
    let g = m.game;
    let me = g.player(m.me);
    let h0 = Hyp::of(g, m.me);
    let t = m.timing_now(false, 0);
    let base = m.value(&h0, &t);
    // The program the plan is aiming at, which should not be spent on
    // building up the "previous programs" bonus.
    let target = h0
        .owned
        .iter()
        .filter_map(|(id, _)| {
            let def = g.config().event(*id)?;
            let (s, c) = m.target_score(def, &h0, &t)?;
            Some((s - c * t.lambda, *id))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|x| x.1);
    let mut out = Vec::new();
    for prog in &me.programs {
        let Some(pv) = g.event_preview(m.me, prog.id, 0) else {
            continue;
        };
        let pre = (pv.breakdown.total() - pv.breakdown.nobles) as f64;
        let mut adj = 0.0;
        if !final_round {
            if !prog.produced && Some(prog.id) != target {
                adj += m.params.new_program;
            }
            // One used token will have to go in the closing ceremonies.
            let mut loss = f64::INFINITY;
            for (a, _) in pv.used.iter() {
                if me.assets.count(a) == 0 {
                    continue;
                }
                let mut h = h0.clone();
                h.assets.remove(a, 1);
                loss = loss.min(base - m.value(&h, &t));
            }
            if loss.is_finite() {
                adj -= loss.max(0.0);
            }
        }
        out.push(Option_ {
            program: prog.id,
            pre,
            adj,
        });
    }
    out
}

#[derive(Clone)]
struct DiceState {
    pos: Vec<u8>,
    used: Vec<bool>,
    moved: Vec<usize>,
    gained: u32,
    medal_supply: u32,
}

fn gen_moves(st: &DiceState, dice: &[DieFace], combined: &[u8]) -> Vec<Action> {
    let mut out = Vec::new();
    let mut seen: Vec<&DieFace> = Vec::new();
    for (die, face) in dice.iter().enumerate() {
        if st.used[die] || seen.contains(&face) {
            continue;
        }
        seen.push(face);
        for noble in (0..st.pos.len()).filter(|n| !st.moved.contains(n)) {
            for &distance in face.values() {
                out.push(Action::MoveNoble { noble, die, distance });
            }
        }
    }
    if dice.len() == 2 && st.moved.is_empty() && st.used.iter().all(|u| !u) {
        for noble in 0..st.pos.len() {
            for &distance in combined {
                out.push(Action::MoveNobleCombined { noble, distance });
            }
        }
    }
    out
}

fn step(m: &Model, st: &DiceState, a: &Action) -> DiceState {
    let board = m.game.rules().board.size as i32;
    let mut s = st.clone();
    let (noble, distance) = match a {
        Action::MoveNoble { noble, die, distance } => {
            s.used[*die] = true;
            (*noble, *distance)
        }
        Action::MoveNobleCombined { noble, distance } => {
            s.used.iter_mut().for_each(|u| *u = true);
            (*noble, *distance)
        }
        _ => return s,
    };
    s.moved.push(noble);
    let to = (s.pos[noble] as i32 + distance as i32).rem_euclid(board) as u8;
    s.pos[noble] = to;
    if m.game.rules().is_resting_place(to) && s.medal_supply > 0 {
        s.medal_supply -= 1;
        s.gained += 1;
    }
    s
}

fn noble_bonus(m: &Model, pos: &[u8]) -> f64 {
    let g = m.game;
    let arena = g.player(m.me).arena;
    let board = g.rules().board.size;
    g.nobles()
        .iter()
        .zip(pos)
        .filter(|(_, p)| arena.contains(**p, board))
        .map(|(n, _)| g.rules().noble_value(n.kind) as f64)
        .sum()
}

fn leaf(m: &Model, opts: &[Option_], st: &DiceState, final_round: bool) -> f64 {
    let nb = noble_bonus(m, &st.pos);
    let medals = m.game.player(m.me).medals + st.gained;
    if final_round {
        let best = opts.iter().map(|o| o.pre + nb).fold(0.0, f64::max);
        best + m.params.medal_value * medals as f64
    } else {
        let lambda = m.lambda();
        let best = opts.iter().map(|o| lambda * (o.pre + nb) + o.adj).fold(0.0, f64::max);
        best + m.params.medal_value * st.gained as f64
    }
}

fn search(m: &Model, opts: &[Option_], st: &DiceState, dice: &[DieFace], combined: &[u8], fin: bool) -> f64 {
    if st.used.iter().all(|u| *u) {
        return leaf(m, opts, st, fin);
    }
    gen_moves(st, dice, combined)
        .iter()
        .map(|a| search(m, opts, &step(m, st, a), dice, combined, fin))
        .fold(f64::NEG_INFINITY, f64::max)
}

pub fn produce(m: &Model, legal: &[Action], pr: &Producing) -> Option<Action> {
    let g = m.game;
    let rules = g.rules();
    let fin = g.round() >= rules.num_rounds;
    let opts = options(m, fin);

    if legal
        .iter()
        .any(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }))
    {
        let st = DiceState {
            pos: g.nobles().iter().map(|n| n.position).collect(),
            used: pr.used.clone(),
            moved: pr.moved.clone(),
            gained: 0,
            medal_supply: g.supply().medals,
        };
        let combined = pr.combined_distances();
        let mut best: Option<(f64, &Action)> = None;
        for a in legal {
            if !matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }) {
                continue;
            }
            let v = search(m, &opts, &step(m, &st, a), &pr.dice, &combined, fin);
            if best.is_none_or(|(b, _)| v > b + 1e-9) {
                best = Some((v, a));
            }
        }
        return best.map(|(_, a)| a.clone());
    }

    let me = g.player(m.me);
    if fin {
        // Spend medals moving nobles into the arena while that pays, then
        // produce with the rest.
        let pos: Vec<u8> = g.nobles().iter().map(|n| n.position).collect();
        let board = rules.board.size as i32;
        let st = DiceState {
            pos: pos.clone(),
            used: vec![true],
            moved: Vec::new(),
            gained: 0,
            medal_supply: 0,
        };
        let current = leaf(m, &opts, &st, true);
        let mut best: Option<(f64, Action)> = None;
        for a in legal {
            if let Action::MedalMoveNoble { noble, delta } = a {
                let mut s = st.clone();
                s.pos[*noble] = (pos[*noble] as i32 + *delta as i32).rem_euclid(board) as u8;
                let v = leaf(m, &opts, &s, true) - m.params.medal_value;
                if v > current + EPS && best.as_ref().is_none_or(|b| v > b.0) {
                    best = Some((v, a.clone()));
                }
            }
        }
        if let Some((_, a)) = best {
            return Some(a);
        }
        let medals = me.medals;
        return legal
            .iter()
            .filter_map(|a| match a {
                Action::ProduceEvent { program, medals: k } if *k == medals => g
                    .event_preview(m.me, *program, *k)
                    .map(|pv| (pv.breakdown.total(), a.clone())),
                _ => None,
            })
            .max_by_key(|x| x.0)
            .map(|x| x.1)
            .or(Some(Action::SkipEvent));
    }

    let lambda = m.lambda();
    let mut best: (f64, Action) = (0.0, Action::SkipEvent);
    for a in legal {
        if let Action::ProduceEvent { program, medals: 0 } = a
            && let Some(pv) = g.event_preview(m.me, *program, 0)
            && let Some(o) = opts.iter().find(|o| o.program == *program)
        {
            let v = lambda * pv.breakdown.total() as f64 + o.adj;
            if v > best.0 {
                best = (v, a.clone());
            }
        }
    }
    Some(best.1)
}

// ---- Closing ceremonies ----------------------------------------------------------

pub fn close(m: &Model, legal: &[Action]) -> Option<Action> {
    let h0 = Hyp::of(m.game, m.me);
    let t = m.timing_now(false, 0);
    let mut best: Option<(f64, &Action)> = None;
    for a in legal {
        let v = match a {
            Action::DiscardAsset { asset } => {
                let mut h = h0.clone();
                h.assets.remove(*asset, 1);
                m.value(&h, &t)
            }
            Action::RequestAsset { asset } => {
                let mut h = h0.clone();
                h.assets.add(*asset, 1);
                m.value(&h, &t)
            }
            _ => continue,
        };
        if best.is_none_or(|(b, _)| v > b + 1e-9) {
            best = Some((v, a));
        }
    }
    best.map(|(_, a)| a.clone())
}
