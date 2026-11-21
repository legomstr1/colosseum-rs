//! One decision function per phase, all priced by [`Ctx::value`].

use crate::model::{Ctx, Hold};
use colosseum_engine::{Acquiring, Action, Producing, Side};

// ---- Investing ---------------------------------------------------------------

/// The best investment among the legal ones, with its gain in points.
fn best_investment(c: &Ctx, me: usize, legal: &[Action], extra_slot: bool) -> Option<(f64, Action)> {
    let g = c.g;
    let t = c.t;
    let pl = &g.players()[me];
    let h0 = Hold::of(pl);
    let mut tm = c.timing(me);
    if extra_slot {
        tm.slots += 1;
    }
    let base = c.value_t(&h0, &tm);
    let mut after = tm;
    after.slots = after.slots.saturating_sub(1);
    let mut best: Option<(f64, Action)> = None;
    let consider = |v: f64, a: Action, best: &mut Option<(f64, Action)>| {
        if best.as_ref().is_none_or(|b| v > b.0) {
            *best = Some((v, a));
        }
    };

    let mut buys: Vec<u8> = Vec::new();
    let mut sides: Vec<Side> = Vec::new();
    let (mut ticket, mut loge) = (false, false);
    if extra_slot {
        // Candidates as if an investment were left.
        for &id in g.programs_for_sale() {
            if t.ev(id).is_some_and(|e| e.cost as i32 <= h0.coins) {
                buys.push(id);
            }
        }
        if h0.arena < t.max_squares && h0.coins >= t.exp_cost as i32 {
            for side in [Side::Left, Side::Right] {
                let sq = pl.arena.expansion_square(side, t.board);
                if g.arena_owner(sq).is_none() {
                    sides.push(side);
                }
            }
        }
        ticket = g.supply().season_tickets > 0 && h0.coins >= t.ticket_cost as i32;
        loge = !h0.loge && g.supply().emperor_loges > 0 && h0.coins >= t.loge_cost as i32;
    } else {
        for a in legal {
            match a {
                Action::BuyProgram { program } => buys.push(*program),
                Action::ExpandArena { side } => sides.push(*side),
                Action::BuySeasonTicket => ticket = true,
                Action::BuildLoge => loge = true,
                _ => {}
            }
        }
    }

    for id in buys {
        let Some(e) = t.ev(id) else { continue };
        let mut h = h0;
        h.coins -= e.cost as i32;
        h.add_program(id);
        consider(c.value_t(&h, &after) - base, Action::BuyProgram { program: id }, &mut best);
    }
    if !sides.is_empty() {
        // Prefer the side whose new square holds, or is about to meet, a noble.
        let mut pick: Option<(f64, Side)> = None;
        for side in sides {
            let sq = pl.arena.expansion_square(side, t.board);
            let mut s = 0.0;
            for (i, n) in g.nobles().iter().enumerate() {
                let dist = (sq as i32 - n.position as i32).rem_euclid(t.board as i32) as f64;
                let v = t.noble_val[i];
                if dist == 0.0 {
                    s += v;
                } else if dist <= 6.0 {
                    s += v / (1.0 + dist);
                }
            }
            if pick.is_none_or(|(b, _)| s > b) {
                pick = Some((s, side));
            }
        }
        if let Some((_, side)) = pick {
            let mut h = h0;
            h.coins -= t.exp_cost as i32;
            h.arena += 1;
            consider(c.value_t(&h, &after) - base, Action::ExpandArena { side }, &mut best);
        }
    }
    if ticket {
        let mut h = h0;
        h.coins -= t.ticket_cost as i32;
        h.tickets += 1;
        consider(c.value_t(&h, &after) - base, Action::BuySeasonTicket, &mut best);
    }
    if loge {
        let mut h = h0;
        h.coins -= t.loge_cost as i32;
        h.loge = true;
        consider(c.value_t(&h, &after) - base, Action::BuildLoge, &mut best);
    }
    best
}

pub fn invest(c: &Ctx, me: usize, legal: &[Action], remaining: u32) -> Action {
    let p = c.p;
    if remaining > 0 {
        if let Some((gain, a)) = best_investment(c, me, legal, false)
            && gain > p.invest_min
        {
            return a;
        }
        return Action::EndInvesting;
    }
    // No investment left: buy another if something is worth more than the
    // SpecialEvent token or the medals it costs.
    if let Some((gain, _)) = best_investment(c, me, legal, true) {
        if legal.contains(&Action::PlaySpecialEvent) && gain > c.special_val() + 0.5 {
            return Action::PlaySpecialEvent;
        }
        if legal.contains(&Action::SpendMedalsForInvestment)
            && gain > c.t.extra_medal_cost as f64 * p.medal_val + 1.0
        {
            return Action::SpendMedalsForInvestment;
        }
    }
    Action::EndInvesting
}

// ---- Acquiring ---------------------------------------------------------------

pub fn acquire(c: &Ctx, me: usize, legal: &[Action], acq: &Acquiring) -> Action {
    let g = c.g;
    let p = c.p;
    let lam = c.lam.max(0.01);
    let coins = g.players()[me].coins;
    let min = c.t.min_bid;
    let markets = g.markets();
    // My gain and the strongest rival's gain for each lot.
    let mut mine = vec![0.0f64; markets.len()];
    let mut rival = vec![0.0f64; markets.len()];
    for (m, lot) in markets.iter().enumerate() {
        if lot.is_empty() {
            continue;
        }
        mine[m] = c.lot_gain(me, lot);
        for q in 0..c.n {
            if q != me && !acq.won[q] {
                rival[m] = rival[m].max(c.lot_gain(q, lot));
            }
        }
    }
    let total = |m: usize| mine[m] + p.deny * rival[m];
    match &acq.auction {
        None => {
            let mut best: Option<(f64, usize)> = None;
            for (m, lot) in markets.iter().enumerate() {
                if lot.is_empty() || !legal.contains(&Action::StartAuction { market: m, bid: min }) {
                    continue;
                }
                let s = total(m);
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, m));
                }
            }
            match best {
                Some((s, market)) if s / lam >= min as f64 => Action::StartAuction { market, bid: min },
                _ => Action::PassInitiative,
            }
        }
        Some(a) => {
            let m = a.market;
            let alt = (0..markets.len())
                .filter(|&j| j != m && !markets[j].is_empty())
                .map(|j| mine[j])
                .fold(0.0f64, f64::max);
            let alt_surplus = (alt - lam * min as f64).max(0.0);
            let cap_pts = total(m) - p.alt_frac * alt_surplus;
            let cap = ((cap_pts / lam).floor().max(0.0) as u32).min(coins);
            let bid = Action::Bid { amount: a.high_bid + 1 };
            if a.high_bid < cap && legal.contains(&bid) {
                bid
            } else {
                Action::PassBid
            }
        }
    }
}

// ---- Producing -----------------------------------------------------------------

struct Opt {
    program: u8,
    /// Spectators without nobles (final round: with every medal spent).
    pre: f64,
    /// Non-final rounds: bonus for a new program, less the discard.
    adj: f64,
}

struct Search<'a> {
    c: &'a Ctx<'a>,
    opts: Vec<Opt>,
    dice: Vec<Vec<u8>>,
    combined: Vec<u8>,
    arena: (u8, u8),
    /// Arenas of rivals who produce after me this round (final round).
    later: Vec<(u8, u8)>,
    fin: bool,
    lamc: f64,
    my_best: f64,
    others_best: f64,
    podium_left: bool,
}

#[derive(Clone, Copy)]
struct DState {
    pos: [u8; 8],
    used: [bool; 2],
    moved: u8,
    gained: u32,
    supply: u32,
}

impl Search<'_> {
    fn nobles_in(&self, pos: &[u8; 8], arena: (u8, u8)) -> f64 {
        let t = self.c.t;
        let mut s = 0.0;
        for (i, v) in t.noble_val.iter().enumerate().take(8) {
            if t.in_arena(arena.0, arena.1, pos[i]) {
                s += v;
            }
        }
        s
    }

    fn event_value(&self, o: &Opt, nb: f64) -> f64 {
        let spect = o.pre + nb;
        if self.fin {
            return spect;
        }
        let mut v = self.lamc * spect + o.adj;
        if self.podium_left && spect.max(self.my_best) > self.others_best && self.my_best <= self.others_best {
            v += self.c.p.podium_val;
        }
        v
    }

    fn leaf(&self, st: &DState) -> f64 {
        let nb = self.nobles_in(&st.pos, self.arena);
        let mut best = 0.0f64;
        for o in &self.opts {
            best = best.max(self.event_value(o, nb));
        }
        let c = self.c;
        if self.fin {
            let mut deny = 0.0;
            for &ar in &self.later {
                deny += self.nobles_in(&st.pos, ar);
            }
            best + st.gained as f64 * c.t.s_medal - c.p.deny_noble * deny
        } else {
            best + st.gained as f64 * c.p.medal_val * c.medal_factor().max(0.5)
        }
    }

    fn step(&self, st: &DState, noble: usize, dist: u8, dice: &[usize]) -> DState {
        let t = self.c.t;
        let mut s = *st;
        for &d in dice {
            s.used[d] = true;
        }
        s.moved |= 1 << noble;
        let to = ((s.pos[noble] as u32 + dist as u32) % t.board as u32) as u8;
        s.pos[noble] = to;
        if t.resting[to as usize] && s.supply > 0 {
            s.supply -= 1;
            s.gained += 1;
        }
        s
    }

    fn best(&self, st: &DState, nn: usize) -> f64 {
        let nd = self.dice.len();
        if (0..nd).all(|d| st.used[d]) {
            return self.leaf(st);
        }
        let mut best = f64::NEG_INFINITY;
        for d in 0..nd {
            if st.used[d] || (d > 0 && !st.used[0] && self.dice[d] == self.dice[0]) {
                continue;
            }
            for noble in 0..nn {
                if st.moved & (1 << noble) != 0 {
                    continue;
                }
                for &dist in &self.dice[d] {
                    best = best.max(self.best(&self.step(st, noble, dist, &[d]), nn));
                }
            }
        }
        if nd == 2 && st.moved == 0 && !st.used[0] && !st.used[1] {
            for noble in 0..nn {
                for &dist in &self.combined {
                    best = best.max(self.leaf(&self.step(st, noble, dist, &[0, 1])));
                }
            }
        }
        if best == f64::NEG_INFINITY { self.leaf(st) } else { best }
    }
}

/// The program my plan is aiming at for the final round.
fn target_program(c: &Ctx, me: usize) -> Option<u8> {
    let h = Hold::of(&c.g.players()[me]);
    let tm = c.timing(me);
    (0..h.np)
        .filter_map(|i| c.target(h.progs[i], &h, &tm).map(|s| (s, h.progs[i])))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|x| x.1)
}

fn build_options(c: &Ctx, me: usize, fin: bool) -> Vec<Opt> {
    let g = c.g;
    let pl = &g.players()[me];
    let pid = colosseum_engine::PlayerId(me);
    let h0 = Hold::of(pl);
    let base = c.value(&h0);
    let target = if fin { None } else { target_program(c, me) };
    let mut out = Vec::new();
    for prog in &pl.programs {
        let Some(pv) = g.event_preview(pid, prog.id, 0) else { continue };
        let mut pre = (pv.breakdown.total() - pv.breakdown.nobles) as f64;
        let mut adj = 0.0;
        if fin {
            pre += pl.medals as f64 * c.t.s_medal;
        } else {
            if !prog.produced && Some(prog.id) != target {
                adj += c.p.new_prog;
            }
            // One used token goes at the closing ceremonies.
            let mut loss = f64::INFINITY;
            for (a, _) in pv.used.iter() {
                let i = a.index();
                if h0.a[i] == 0 {
                    continue;
                }
                let mut h = h0;
                h.a[i] -= 1;
                loss = loss.min(base - c.value(&h));
            }
            if loss.is_finite() {
                adj -= loss.max(0.0);
            }
        }
        out.push(Opt {
            program: prog.id,
            pre,
            adj,
        });
    }
    out
}

pub fn produce(c: &Ctx, me: usize, legal: &[Action], pr: &Producing) -> Action {
    let g = c.g;
    let t = c.t;
    let p = c.p;
    let fin = c.last;
    let pl = &g.players()[me];

    let dice_phase = legal
        .iter()
        .any(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }));
    if !dice_phase {
        // Lock in a medal from a SpecialEvent token when it's time.
        if legal.contains(&Action::ExchangeSpecialEventForMedal)
            && (fin || (c.medal_supply as f64) <= p.se_lock)
        {
            return Action::ExchangeSpecialEventForMedal;
        }
    }

    if dice_phase && (pr.dice.len() > 2 || g.nobles().len() > 8) {
        // Outside what the search models: take the first dice move.
        return legal
            .iter()
            .find(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }))
            .cloned()
            .unwrap_or_else(|| legal[0].clone());
    }
    let opts = build_options(c, me, fin);
    let n = c.n;
    let si = c.seat_index(me);
    let later: Vec<(u8, u8)> = if fin {
        (0..n)
            .filter(|&q| q != me && c.seat_index(q) > si)
            .map(|q| (g.players()[q].arena.start, g.players()[q].arena.len))
            .collect()
    } else {
        Vec::new()
    };
    let others_best = (0..n)
        .filter(|&q| q != me)
        .map(|q| g.players()[q].best_score)
        .max()
        .unwrap_or(0) as f64;
    let s = Search {
        c,
        opts,
        dice: pr.dice.iter().map(|f| f.values().to_vec()).collect(),
        combined: pr.combined_distances(),
        arena: (pl.arena.start, pl.arena.len),
        later,
        fin,
        lamc: c.lam,
        my_best: pl.best_score as f64,
        others_best,
        podium_left: g.supply().podiums > 0,
    };
    let nn = g.nobles().len().min(8);
    let mut pos = [0u8; 8];
    for (i, nob) in g.nobles().iter().take(8).enumerate() {
        pos[i] = nob.position;
    }
    let mut moved = 0u8;
    for &m in &pr.moved {
        if m < 8 {
            moved |= 1 << m;
        }
    }
    let st = DState {
        pos,
        used: [pr.used.first().copied().unwrap_or(true), pr.used.get(1).copied().unwrap_or(true)],
        moved,
        gained: 0,
        supply: c.medal_supply,
    };

    if dice_phase {
        let mut best: Option<(f64, &Action)> = None;
        for a in legal {
            let v = match a {
                Action::MoveNoble { noble, die, distance } if *noble < 8 => s.best(&s.step(&st, *noble, *distance, &[*die]), nn),
                Action::MoveNobleCombined { noble, distance } if *noble < 8 => s.leaf(&s.step(&st, *noble, *distance, &[0, 1])),
                _ => continue,
            };
            if best.is_none_or(|(b, _)| v > b + 1e-9) {
                best = Some((v, a));
            }
        }
        return best.map_or_else(|| legal[0].clone(), |(_, a)| a.clone());
    }

    if fin {
        // Spend a medal moving a noble when the spectators beat the medal.
        let cur = s.leaf(&st);
        let mut best: Option<(f64, Action)> = None;
        for a in legal {
            if let Action::MedalMoveNoble { noble, delta } = a
                && *noble < 8
            {
                let mut s2 = st;
                s2.pos[*noble] = (s2.pos[*noble] as i32 + *delta as i32).rem_euclid(t.board as i32) as u8;
                let v = s.leaf(&s2) - t.s_medal;
                if v > cur + 0.01 && best.as_ref().is_none_or(|b| v > b.0) {
                    best = Some((v, a.clone()));
                }
            }
        }
        if let Some((_, a)) = best {
            return a;
        }
        let medals = pl.medals;
        let pid = colosseum_engine::PlayerId(me);
        return legal
            .iter()
            .filter_map(|a| match a {
                Action::ProduceEvent { program, medals: k } if *k == medals => g
                    .event_preview(pid, *program, *k)
                    .map(|pv| (pv.breakdown.total(), a.clone())),
                _ => None,
            })
            .max_by_key(|x| x.0)
            .map_or(Action::SkipEvent, |x| x.1);
    }

    let nb = s.nobles_in(&st.pos, s.arena);
    let mut best: (f64, Action) = (0.0, Action::SkipEvent);
    for o in &s.opts {
        let a = Action::ProduceEvent {
            program: o.program,
            medals: 0,
        };
        let v = s.event_value(o, nb);
        if v > best.0 && legal.contains(&a) {
            best = (v, a);
        }
    }
    best.1
}

// ---- Closing ---------------------------------------------------------------------

pub fn close(c: &Ctx, me: usize, legal: &[Action]) -> Action {
    let g = c.g;
    let h0 = Hold::of(&g.players()[me]);
    let base = c.value(&h0);
    let donor = match g.phase() {
        colosseum_engine::Phase::Closing(cl) => cl.donation.map(|(_, d)| d.0),
        _ => None,
    };
    let mut best: Option<(f64, &Action)> = None;
    for a in legal {
        let v = match a {
            Action::DiscardAsset { asset } => {
                let mut h = h0;
                let i = asset.index();
                h.a[i] = h.a[i].saturating_sub(1);
                c.value(&h) - base
            }
            Action::RequestAsset { asset } => {
                let i = asset.index();
                let mut h = h0;
                h.a[i] = h.a[i].saturating_add(1);
                let mut v = c.value(&h) - base;
                if let Some(d) = donor {
                    let hd0 = Hold::of(&g.players()[d]);
                    let mut hd = hd0;
                    hd.a[i] = hd.a[i].saturating_sub(1);
                    v += c.p.deny * (c.value(&hd0) - c.value(&hd)).max(0.0);
                }
                v
            }
            _ => continue,
        };
        if best.is_none_or(|(b, _)| v > b + 1e-9) {
            best = Some((v, a));
        }
    }
    best.map_or_else(|| legal[0].clone(), |(_, a)| a.clone())
}
