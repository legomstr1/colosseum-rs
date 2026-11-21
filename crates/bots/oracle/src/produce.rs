//! Producing: an exhaustive search over the dice (on plain arrays, no game
//! clones), with a leaf that values this round's event, medals earned, and in
//! the final round the estimated chance of winning against everyone still
//! to produce.

use crate::model::{Ctx, Hyp, NA};
use colosseum_engine::{Action, Asset, Producing};

const MAXN: usize = 8;

/// Static facts for one search.
struct Search<'a, 'b> {
    c: &'a Ctx<'b>,
    board: u8,
    nn: usize,
    dice: Vec<Vec<u8>>,
    combined: Vec<u8>,
    /// Value of my event choices before nobles, best first: (value, program).
    /// Non-final: value in points of producing it now, excluding nobles.
    opts: Vec<(f64, u8, u32)>,
    /// Points per noble spectator in my event.
    nb_w: f64,
    medal_w: f64,
    /// Final round: opponents still to produce, with their best producible
    /// score now (before nobles) and their best score so far.
    later: Vec<(usize, f64, f64)>,
    /// Final round: best scores of players already done (and their totals).
    done_best: Vec<f64>,
    my_medals: u32,
    my_best: f64,
    /// Non-final rounds: the podium race. Best scores of players who already
    /// produced this round, and (player, producible now, best so far) for
    /// the ones still to produce.
    pod_done: Vec<f64>,
    pod_later: Vec<(usize, f64, f64)>,
    pod_w: f64,
}

#[derive(Clone, Copy)]
struct St {
    pos: [u8; MAXN],
    used: u8,
    moved: u8,
    gained: u32,
}

impl Search<'_, '_> {
    fn in_arena(&self, who: usize, sq: u8) -> bool {
        self.c.g.players()[who].arena.contains(sq, self.board)
    }

    fn arena_nobles(&self, who: usize, pos: &[u8; MAXN]) -> u32 {
        let mut s = 0;
        for (&sq, &v) in pos.iter().zip(&self.c.t.noble_vals).take(self.nn) {
            if self.in_arena(who, sq) {
                s += v;
            }
        }
        s
    }

    /// Final round: best extra spectators from spending medals to pull nobles
    /// into my arena instead of on the event; returns (gain, positions after).
    fn medal_pulls(&self, pos: &[u8; MAXN], medals: u32) -> (u32, [u8; MAXN]) {
        let me = self.c.me.0;
        let bonus = self.c.g.rules().scoring.medal_bonus;
        let reach = self.c.g.rules().medals.max_noble_move as i32;
        let mut cands: Vec<(u32, usize, u8)> = Vec::new();
        for (i, (&sq, &v)) in pos.iter().zip(&self.c.t.noble_vals).take(self.nn).enumerate() {
            if self.in_arena(me, sq) {
                continue;
            }
            if v <= bonus {
                continue;
            }
            let mut best: Option<u8> = None;
            for d in 1..=reach {
                for s in [d, -d] {
                    let to = (pos[i] as i32 + s).rem_euclid(self.board as i32) as u8;
                    if best.is_none() && self.in_arena(me, to) {
                        best = Some(to);
                    }
                }
            }
            if let Some(to) = best {
                cands.push((v, i, to));
            }
        }
        cands.sort_by(|a, b| b.0.cmp(&a.0));
        let mut out = *pos;
        let mut gain = 0;
        for (v, i, to) in cands.into_iter().take(medals as usize) {
            out[i] = to;
            gain += v - bonus;
        }
        (gain, out)
    }

    /// Value of the podium chance if my best score becomes `score`.
    fn podium(&self, score: f64, pos: &[u8; MAXN]) -> f64 {
        if self.pod_w == 0.0 {
            return 0.0;
        }
        let tau4 = 4.0 * self.c.p.podium_tau;
        let ramp = |x: f64| (0.5 + x / tau4).clamp(0.0, 1.0);
        let mut p = 1.0;
        for &b in &self.pod_done {
            p *= ramp(score - b);
        }
        for &(j, now, best) in &self.pod_later {
            let est = (now + self.arena_nobles(j, pos) as f64 + self.c.p.opp_extra).max(best);
            p *= ramp(score - est);
        }
        self.pod_w * p
    }

    /// Non-final rounds: the best event to produce with nobles at `pos`, and
    /// its value (skipping is 0 plus the podium chance as things stand).
    fn choose_event(&self, pos: &[u8; MAXN]) -> (f64, Option<u8>) {
        let nb = self.arena_nobles(self.c.me.0, pos);
        let mut best = (self.podium(self.my_best, pos), None);
        for &(v, id, s) in &self.opts {
            let total = v + self.nb_w * nb as f64 + self.podium(self.my_best.max((s + nb) as f64), pos);
            if total > best.0 {
                best = (total, Some(id));
            }
        }
        best
    }

    fn leaf(&self, st: &St) -> f64 {
        if self.later.is_empty() && self.done_best.is_empty() {
            // Not the final round.
            return self.choose_event(&st.pos).0 + self.medal_w * st.gained as f64;
        }
        self.final_leaf(&st.pos, self.my_medals + st.gained, true)
    }

    /// Final round: my exact score with nobles at `pos` and `medals` to spend
    /// (pulling nobles in with them first if `pulls`), as a win chance.
    fn final_leaf(&self, pos0: &[u8; MAXN], medals: u32, pulls: bool) -> f64 {
        let me = self.c.me.0;
        let (pull, pos) = if pulls { self.medal_pulls(pos0, medals) } else { (0, *pos0) };
        let nb = self.arena_nobles(me, &pos);
        let bonus = self.c.g.rules().scoring.medal_bonus;
        let mine = self
            .opts
            .iter()
            .map(|(v, _, _)| *v + (nb + pull + medals * bonus) as f64)
            .fold(0.0f64, f64::max)
            .max(self.my_best);
        let tau = self.c.p.tau;
        let mut p = 1.0;
        for &b in &self.done_best {
            p *= sig((mine - b) / (0.25 * tau));
        }
        for &(j, now, best) in &self.later {
            let est = (now + self.arena_nobles(j, &pos) as f64 + self.c.p.opp_extra).max(best);
            p *= sig((mine - est) / tau);
        }
        // Tie-break towards my own score.
        self.c.p.final_wp * p + 1e-4 * mine
    }

    fn search(&self, st: &St) -> f64 {
        let nd = self.dice.len();
        if (0..nd).all(|d| st.used & (1 << d) != 0) {
            return self.leaf(st);
        }
        let mut best = f64::NEG_INFINITY;
        for d in 0..nd {
            if st.used & (1 << d) != 0 {
                continue;
            }
            for nb in 0..self.nn {
                if st.moved & (1 << nb) != 0 {
                    continue;
                }
                for &dist in &self.dice[d] {
                    let mut s = *st;
                    s.used |= 1 << d;
                    self.step(&mut s, nb, dist);
                    best = best.max(self.search(&s));
                }
            }
        }
        if nd == 2 && st.used == 0 && st.moved == 0 {
            for nb in 0..self.nn {
                for &dist in &self.combined {
                    let mut s = *st;
                    s.used = 0b11;
                    self.step(&mut s, nb, dist);
                    best = best.max(self.search(&s));
                }
            }
        }
        if best == f64::NEG_INFINITY { self.leaf(st) } else { best }
    }

    fn step(&self, s: &mut St, nb: usize, dist: u8) {
        s.moved |= 1 << nb;
        let to = ((s.pos[nb] as u32 + dist as u32) % self.board as u32) as u8;
        s.pos[nb] = to;
        if self.c.t.resting.get(to as usize).copied().unwrap_or(false) && s.gained < self.c.medal_supply {
            s.gained += 1;
        }
    }
}

fn sig(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Best discard after producing with `used`, and the value lost.
pub fn discard_loss(c: &Ctx, h: &Hyp, used: &[u32; NA]) -> (f64, Option<usize>) {
    let me = c.me.0;
    let tm = c.timing_at(4, 0);
    let base = c.value(me, h, &tm, None);
    let mut best: Option<(f64, usize)> = None;
    for (a, &u) in used.iter().enumerate() {
        if u == 0 || h.a[a] == 0 {
            continue;
        }
        let mut h2 = *h;
        h2.a[a] -= 1;
        let loss = base - c.value(me, &h2, &tm, None);
        if best.is_none_or(|b| loss < b.0) {
            best = Some((loss, a));
        }
    }
    match best {
        Some((l, a)) => (l.max(0.0), Some(a)),
        None => (0.0, None),
    }
}

/// Tokens used by producing `prog` with holding `h` (Jokers included).
pub fn used_tokens(h: &Hyp, req: &[u32; NA]) -> [u32; NA] {
    let mut used = [0u32; NA];
    let mut missing = 0;
    for a in 0..NA {
        let u = h.a[a].min(req[a]);
        used[a] = u;
        missing += req[a] - u;
    }
    let j = Asset::Joker.index();
    used[j] = h.a[j].min(missing);
    used
}

/// Non-final event options: (value now excluding nobles, program, spectators
/// excluding nobles).
fn options(c: &Ctx, h: &Hyp) -> Vec<(f64, u8, u32)> {
    let me = c.me.0;
    let tm_after = c.timing_at(4, 0);
    let skip_val = c.value(me, h, &tm_after, None);
    let my_best = c.g.players()[me].best_score as f64;
    let mut out = Vec::new();
    for i in 0..h.np {
        let Some(prog) = c.t.prog(h.progs[i]) else { continue };
        let Some(s) = c.event_score(me, h, prog, 0, 0) else { continue };
        let mut h2 = *h;
        h2.set_produced(prog.id);
        let used = used_tokens(h, &prog.req);
        let (_, d) = discard_loss(c, &h2, &used);
        if let Some(a) = d {
            h2.a[a] -= 1;
        }
        h2.coins += s;
        let v = c.value(me, &h2, &tm_after, None) - skip_val;
        let v = v + c.p.best_w * (s as f64 - my_best).max(0.0);
        out.push((v, prog.id, s));
    }
    out
}

pub fn produce(c: &Ctx, legal: &[Action], pr: &Producing) -> Action {
    let me = c.me.0;
    let g = c.g;
    let h = c.hyp(me);
    // The search tracks dice in a byte; more dice than that just takes the
    // first legal move.
    let dice_phase = legal
        .iter()
        .any(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }));
    if dice_phase && pr.dice.len() > 7 {
        return legal[0].clone();
    }

    if c.last && !dice_phase && legal.contains(&Action::ExchangeSpecialEventForMedal) {
        return Action::ExchangeSpecialEventForMedal;
    }

    // Marginal worth of a spectator (a coin) after this production.
    let tm4 = c.timing_at(4, 0);
    let coins = h.coins as f64;
    let lambda = c.coin_pts(coins + 1.0, &tm4) - c.coin_pts(coins, &tm4);
    let nn = g.nobles().len().min(MAXN);
    let mut pos = [0u8; MAXN];
    for (i, n) in g.nobles().iter().take(MAXN).enumerate() {
        pos[i] = n.position;
    }

    let mut pod_done = Vec::new();
    let mut pod_later = Vec::new();
    if !c.last && g.supply().podiums > 0 {
        for k in 0..c.n {
            let j = g.seat(k).0;
            if j == me {
                continue;
            }
            let best = g.players()[j].best_score as f64;
            if k > pr.turn {
                let hj = c.hyp(j);
                let now = c.best_now(j, &hj).map_or(0.0, |x| x.0 as f64);
                pod_later.push((j, now, best));
            } else {
                pod_done.push(best);
            }
        }
    }
    let (opts, later, done_best) = if c.last {
        let mut opts = Vec::new();
        for i in 0..h.np {
            if let Some(prog) = c.t.prog(h.progs[i])
                && let Some(s) = c.event_score(me, &h, prog, 0, 0)
            {
                opts.push((s as f64, prog.id, s));
            }
        }
        let mut later = Vec::new();
        let mut done = Vec::new();
        let turn = pr.turn;
        for k in 0..c.n {
            let j = g.seat(k).0;
            if j == me {
                continue;
            }
            let best = g.players()[j].best_score as f64;
            if k > turn {
                let hj = c.hyp(j);
                let now = c.best_now(j, &hj).map_or(0.0, |x| x.0 as f64 + hj.medals as f64 * 3.0);
                later.push((j, now, best));
            } else {
                done.push(best);
            }
        }
        (opts, later, done)
    } else {
        (options(c, &h), Vec::new(), Vec::new())
    };
    let s = Search {
        c,
        board: g.rules().board.size,
        nn,
        dice: pr.dice.iter().map(|f| f.values().to_vec()).collect(),
        combined: pr.combined_distances(),
        opts,
        nb_w: lambda,
        medal_w: c.p.medal_value,
        later,
        done_best,
        my_medals: h.medals,
        my_best: g.players()[me].best_score as f64,
        pod_w: if pod_done.is_empty() && pod_later.is_empty() { 0.0 } else { c.p.podium_w },
        pod_done,
        pod_later,
    };

    if dice_phase {
        let mut st = St {
            pos,
            used: 0,
            moved: 0,
            gained: 0,
        };
        for (d, u) in pr.used.iter().enumerate() {
            if *u {
                st.used |= 1 << d;
            }
        }
        for &m in &pr.moved {
            if m < MAXN {
                st.moved |= 1 << m;
            }
        }
        let mut best: Option<(f64, &Action)> = None;
        for a in legal {
            let mut s2 = st;
            match a {
                Action::MoveNoble { noble, die, distance } if *noble < nn && *die < 8 => {
                    s2.used |= 1 << die;
                    s.step(&mut s2, *noble, *distance);
                }
                Action::MoveNobleCombined { noble, distance } if *noble < nn => {
                    s2.used = u8::MAX;
                    s.step(&mut s2, *noble, *distance);
                }
                _ => continue,
            }
            let v = s.search(&s2);
            if best.is_none_or(|b| v > b.0 + 1e-12) {
                best = Some((v, a));
            }
        }
        if let Some((_, a)) = best {
            return a.clone();
        }
    }

    if c.last {
        // Pull high-value nobles in with medals, then produce with the rest.
        let (gain, after) = s.medal_pulls(&pos, h.medals);
        if gain > 0 {
            for i in 0..nn {
                if after[i] != pos[i] {
                    let board = s.board as i32;
                    let mut delta = (after[i] as i32 - pos[i] as i32).rem_euclid(board);
                    if delta > board / 2 {
                        delta -= board;
                    }
                    let a = Action::MedalMoveNoble {
                        noble: i,
                        delta: delta as i8,
                    };
                    if legal.contains(&a) {
                        return a;
                    }
                }
            }
        }
        let mut best: Option<(u32, Action)> = None;
        for a in legal {
            if let Action::ProduceEvent { program, medals } = a
                && *medals == h.medals
                && let Some(pv) = g.event_preview(c.me, *program, *medals)
            {
                let t = pv.breakdown.total();
                if best.as_ref().is_none_or(|b| t > b.0) {
                    best = Some((t, a.clone()));
                }
            }
        }
        return best.map_or(Action::SkipEvent, |b| b.1);
    }

    // Non-final: produce the best option (medals kept for later).
    match s.choose_event(&pos).1 {
        Some(program) => {
            let a = Action::ProduceEvent { program, medals: 0 };
            if legal.contains(&a) { a } else { Action::SkipEvent }
        }
        None => Action::SkipEvent,
    }
}
