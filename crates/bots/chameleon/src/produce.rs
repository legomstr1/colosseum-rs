//! Producing: noble moves (searched exhaustively over the dice on plain
//! arrays) and the choice of event. In the final round the search also
//! counts the nobles it leaves in threatening rivals' arenas against me.

use crate::model::{Hyp, Model};
use colosseum_engine::{Action, Arena, DieFace, Phase, PlayerId, Producing};

const MAXN: usize = 8;

struct Opt {
    /// Score without nobles.
    pre: f64,
    /// Non-final adjustment (new program bonus, discard loss...).
    adj: f64,
}

#[derive(Clone, Copy)]
struct St {
    pos: [u8; MAXN],
    used: [bool; 4],
    moved: u8,
    gained: u32,
    supply: u32,
}

struct Ctx<'a> {
    m: &'a Model<'a>,
    n: usize,
    board: u8,
    values: [f64; MAXN],
    rest: [bool; 64],
    arena: Arena,
    opts: Vec<Opt>,
    fin: bool,
    lambda: f64,
    medals: u32,
    /// Best score so far among my rivals, my own, and a podium's worth to me.
    podium: (f64, f64, f64),
    /// Arenas of rivals still to produce in the final round, with weights.
    rivals: Vec<(Arena, f64)>,
    dice: Vec<Vec<u8>>,
    combined: Vec<u8>,
}

impl Ctx<'_> {
    fn nb(&self, pos: &[u8; MAXN], arena: Arena) -> f64 {
        let mut s = 0.0;
        for (p, v) in pos.iter().zip(&self.values).take(self.n) {
            if arena.contains(*p, self.board) {
                s += v;
            }
        }
        s
    }

    /// Value of the podium chance an event of `total` gives (non-final rounds).
    fn pod(&self, total: f64) -> f64 {
        let (others, mine, v) = self.podium;
        if v <= 0.0 {
            return 0.0;
        }
        let before = mine > others;
        let after = total.max(mine) > others;
        if after && !before { v } else { 0.0 }
    }

    fn leaf(&self, st: &St) -> f64 {
        let nb = self.nb(&st.pos, self.arena);
        let mv = self.m.prm.medal_value;
        if self.fin {
            let best = self.opts.iter().map(|o| o.pre + nb).fold(0.0, f64::max);
            let mut deny = 0.0;
            for (a, w) in &self.rivals {
                deny += w * self.nb(&st.pos, *a);
            }
            best + mv * (self.medals + st.gained) as f64 - deny
        } else {
            let best = self
                .opts
                .iter()
                .map(|o| self.lambda * (o.pre + nb) + o.adj + self.pod(o.pre + nb))
                .fold(0.0, f64::max);
            best + mv * st.gained as f64
        }
    }

    fn step(&self, st: &St, noble: usize, dist: u8, dice: &[usize]) -> St {
        let mut s = *st;
        for &d in dice {
            s.used[d] = true;
        }
        s.moved |= 1 << noble;
        let to = ((s.pos[noble] as u32 + dist as u32) % self.board as u32) as u8;
        s.pos[noble] = to;
        if (to as usize) < 64 && self.rest[to as usize] && s.supply > 0 {
            s.supply -= 1;
            s.gained += 1;
        }
        s
    }

    fn search(&self, st: &St) -> f64 {
        let nd = self.dice.len();
        if (0..nd).all(|d| st.used[d]) {
            return self.leaf(st);
        }
        let mut best = f64::NEG_INFINITY;
        for d in 0..nd {
            if st.used[d] {
                continue;
            }
            for noble in 0..self.n {
                if st.moved & (1 << noble) != 0 {
                    continue;
                }
                for &dist in &self.dice[d] {
                    best = best.max(self.search(&self.step(st, noble, dist, &[d])));
                }
            }
        }
        if nd == 2 && st.moved == 0 && !st.used[0] && !st.used[1] {
            for noble in 0..self.n {
                for &dist in &self.combined {
                    best = best.max(self.leaf(&self.step(st, noble, dist, &[0, 1])));
                }
            }
        }
        if best == f64::NEG_INFINITY { self.leaf(st) } else { best }
    }
}

fn options(m: &Model, me: PlayerId, fin: bool) -> Vec<Opt> {
    let g = m.g;
    let pl = g.player(me);
    let h0 = Hyp::of(g, me);
    let t = m.timing(me);
    let base = m.value(me.0, &h0, &t);
    // The program the plan aims at: don't spend it on the "previous" bonus.
    let target = h0
        .programs()
        .iter()
        .filter_map(|(id, _)| {
            let (def, _) = m.def(*id)?;
            let (s, c) = m.target_score(me.0, def, &h0, &t)?;
            Some((s - c * t.lambda, *id))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|x| x.1);
    let mut out = Vec::new();
    for prog in &pl.programs {
        let Some(pv) = g.event_preview(me, prog.id, 0) else {
            continue;
        };
        let pre = (pv.breakdown.total() - pv.breakdown.nobles) as f64;
        let mut adj = 0.0;
        if !fin {
            if !prog.produced && Some(prog.id) != target {
                adj += m.prm.new_program;
            }
            let mut loss = f64::INFINITY;
            for (a, _) in pv.used.iter() {
                if pl.assets.count(a) == 0 {
                    continue;
                }
                let mut h = h0.clone();
                h.assets.remove(a, 1);
                loss = loss.min(base - m.value(me.0, &h, &t));
            }
            if loss.is_finite() {
                adj -= loss.max(0.0);
            }
        }
        out.push(Opt { pre, adj });
    }
    out
}

pub fn produce(m: &Model, me: PlayerId, legal: &[Action], pr: &Producing) -> Action {
    let g = m.g;
    let rules = g.rules();
    let fin = g.round() >= rules.num_rounds;
    let pl = g.player(me);

    if fin && legal.contains(&Action::ExchangeSpecialEventForMedal) {
        return Action::ExchangeSpecialEventForMedal;
    }

    let has_dice = legal
        .iter()
        .any(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }));
    let n = g.nobles().len().min(MAXN);
    let mut pos = [0u8; MAXN];
    let mut values = [0.0; MAXN];
    for (i, nb) in g.nobles().iter().take(MAXN).enumerate() {
        pos[i] = nb.position;
        values[i] = rules.noble_value(nb.kind) as f64;
    }
    let mut rest = [false; 64];
    for (sq, _) in &rules.noble_positions {
        if (*sq as usize) < 64 {
            rest[*sq as usize] = true;
        }
    }
    let mut rivals = Vec::new();
    if fin {
        // Rivals still to produce this round whose final event could beat mine.
        let mine = m.projection(me);
        let np = g.num_players();
        let my_turn = (me.0 + np - g.start_player().0) % np;
        for q in g.players() {
            let qt = (q.id.0 + np - g.start_player().0) % np;
            if q.id == me || qt <= my_turn {
                continue;
            }
            let pq = m.projection(q.id);
            let w = if pq >= mine - 6.0 {
                0.6
            } else if pq >= mine - 15.0 {
                0.25
            } else {
                0.0
            };
            if w > 0.0 {
                rivals.push((q.arena, w * m.prm.final_deny));
            }
        }
    }
    let ctx = Ctx {
        m,
        n,
        board: rules.board.size,
        values,
        rest,
        arena: pl.arena,
        opts: options(m, me, fin),
        fin,
        lambda: m.lambda(),
        podium: {
            let others = g
                .players()
                .iter()
                .filter(|q| q.id != me)
                .map(|q| q.best_score)
                .max()
                .unwrap_or(0) as f64;
            let v = if !fin && g.supply().podiums > 0 {
                m.prm.podium_w * rules.scoring.podium_bonus as f64
            } else {
                0.0
            };
            (others, pl.best_score as f64, v)
        },
        medals: pl.medals,
        rivals,
        dice: pr.dice.iter().map(|f: &DieFace| f.values().to_vec()).collect(),
        combined: pr.combined_distances(),
    };
    let mut used = [true; 4];
    for (i, u) in pr.used.iter().take(4).enumerate() {
        used[i] = *u;
    }
    let mut moved = 0u8;
    for &i in &pr.moved {
        if i < 8 {
            moved |= 1 << i;
        }
    }
    let st = St {
        pos,
        used,
        moved,
        gained: 0,
        supply: g.supply().medals,
    };

    if has_dice && ctx.dice.len() <= 4 {
        let mut best: Option<(f64, &Action)> = None;
        for a in legal {
            let next = match a {
                Action::MoveNoble { noble, die, distance } if *noble < n => ctx.step(&st, *noble, *distance, &[*die]),
                Action::MoveNobleCombined { noble, distance } if *noble < n && ctx.dice.len() >= 2 => {
                    ctx.step(&st, *noble, *distance, &[0, 1])
                }
                _ => continue,
            };
            let v = ctx.search(&next);
            if best.is_none_or(|(b, _)| v > b + 1e-9) {
                best = Some((v, a));
            }
        }
        if let Some((_, a)) = best {
            return a.clone();
        }
    }

    if fin {
        // Medal moves that pay for themselves (my nobles, or a rival's).
        if pl.medals > 0 {
            let current = ctx.leaf(&St { used: [true; 4], ..st });
            let board = rules.board.size as i32;
            let mut best: Option<(f64, &Action)> = None;
            for a in legal {
                if let Action::MedalMoveNoble { noble, delta } = a
                    && *noble < n
                {
                    let mut s = St { used: [true; 4], ..st };
                    s.pos[*noble] = (pos[*noble] as i32 + *delta as i32).rem_euclid(board) as u8;
                    let v = ctx.leaf(&s) - m.prm.medal_value;
                    if v > current + 0.05 && best.is_none_or(|(b, _)| v > b) {
                        best = Some((v, a));
                    }
                }
            }
            if let Some((_, a)) = best {
                return a.clone();
            }
        }
        let medals = pl.medals;
        return legal
            .iter()
            .filter_map(|a| match a {
                Action::ProduceEvent { program, medals: k } if *k == medals => g
                    .event_preview(me, *program, *k)
                    .map(|pv| (pv.breakdown.total(), a)),
                _ => None,
            })
            .max_by_key(|x| x.0)
            .map(|x| x.1.clone())
            .unwrap_or(Action::SkipEvent);
    }

    let lambda = m.lambda();
    let best_so_far = pl.best_score as f64;
    let mut best: (f64, Action) = (0.0, Action::SkipEvent);
    let mut oi = 0;
    for prog in &pl.programs {
        let Some(pv) = g.event_preview(me, prog.id, 0) else {
            continue;
        };
        let Some(o) = ctx.opts.get(oi) else {
            break;
        };
        oi += 1;
        let a = Action::ProduceEvent { program: prog.id, medals: 0 };
        if !legal.contains(&a) {
            continue;
        }
        let total = pv.breakdown.total() as f64;
        let v = lambda * total + o.adj + m.prm.best_bump * (total - best_so_far).max(0.0) + ctx.pod(total);
        if v > best.0 {
            best = (v, a);
        }
    }
    if matches!(g.phase(), Phase::Producing(_)) { best.1 } else { Action::SkipEvent }
}
