//! Producing-phase planner: searches noble moves (dice, combined, medal)
//! and the final event choice using plain arithmetic on noble positions.

use colosseum_engine::{Action, Arena, DieFace, Game, NobleKind, PlayerId};

const MEDAL_KEEP_VALUE: f64 = 2.5;
const MAX_MEDAL_MOVES: usize = 3;
const NODE_BUDGET: usize = 40_000;

#[derive(Clone)]
struct State {
    pos: Vec<u8>,
    used: Vec<bool>,
    moved: Vec<usize>,
    medals: u32,
    supply: u32,
    medal_moves: usize,
    can_combine: bool,
}

struct Ctx {
    me: PlayerId,
    board: u8,
    final_round: bool,
    kinds: Vec<NobleKind>,
    values: Vec<u32>,
    arenas: Vec<Arena>,
    /// (player index, produces later this round, base score without nobles, fixed best score)
    opps: Vec<Opp>,
    /// (program id, score without nobles and medals, unproduced)
    progs: Vec<(u8, u32, bool)>,
    dice: Vec<DieFace>,
    rest: Vec<bool>,
    medal_bonus: u32,
    max_move: i8,
    floor: u32,
    nodes: usize,
}

struct Opp {
    idx: usize,
    later: bool,
    base: u32,
    weight: f64,
}

impl Ctx {
    fn nobles_in(&self, arena: &Arena, pos: &[u8]) -> u32 {
        (0..pos.len())
            .filter(|&i| arena.contains(pos[i], self.board))
            .map(|i| self.values[i])
            .sum()
    }

    fn leaf(&mut self, st: &State) -> (f64, Action) {
        self.nodes += 1;
        let my_nob = self.nobles_in(&self.arenas[self.me.0], &st.pos);
        let spend = if self.final_round { st.medals } else { 0 };
        let mut best: Option<(f64, Action)> = None;
        let mut s_best = 0u32;
        for &(prog, base, fresh) in &self.progs {
            let s = base + my_nob + spend * self.medal_bonus;
            let mut v = s as f64;
            if !self.final_round && fresh {
                v += 3.0;
            }
            if best.as_ref().is_none_or(|b| v > b.0) {
                best = Some((v, Action::ProduceEvent { program: prog, medals: spend }));
                s_best = s;
            }
        }
        let (mut v, act) = best.unwrap_or((0.0, Action::SkipEvent));
        if !self.final_round {
            v += st.medals as f64 * MEDAL_KEEP_VALUE;
        }
        // Opponents' noble gains from where we leave the nobles.
        let mut worst_later = self.floor as f64;
        for o in 0..self.opps.len() {
            let (idx, later, base, weight) = {
                let o = &self.opps[o];
                (o.idx, o.later, o.base, o.weight)
            };
            let n = self.nobles_in(&self.arenas[idx], &st.pos);
            if self.final_round {
                if later {
                    worst_later = worst_later.max((base + n) as f64);
                }
            } else {
                v -= n as f64 * weight * if later { 1.0 } else { 0.4 };
            }
        }
        if self.final_round {
            let s = s_best as f64;
            // Reward topping the field; punish letting a rival top us.
            v = s - 0.7 * (worst_later - s).max(0.0) + 0.02 * st.medals as f64;
        }
        (v, act)
    }

    fn successors(&self, st: &State) -> Vec<(Action, State)> {
        let mut out = Vec::new();
        let board = self.board as i32;
        let dice_left = st.used.iter().any(|u| !u);
        if dice_left {
            let mut seen: Vec<&DieFace> = Vec::new();
            for (die, face) in self.dice.iter().enumerate() {
                if st.used[die] || seen.contains(&face) {
                    continue;
                }
                seen.push(face);
                for noble in (0..self.kinds.len()).filter(|n| !st.moved.contains(n)) {
                    for &distance in face.values() {
                        let mut ns = st.clone();
                        ns.used[die] = true;
                        ns.moved.push(noble);
                        self.shift(&mut ns, noble, distance as i32, true, board);
                        out.push((Action::MoveNoble { noble, die, distance }, ns));
                    }
                }
            }
            if self.dice.len() == 2 && st.moved.is_empty() && st.can_combine {
                let mut sums = vec![0u8];
                for face in &self.dice {
                    sums = sums
                        .iter()
                        .flat_map(|s| face.values().iter().map(move |v| s + v))
                        .collect();
                }
                sums.sort_unstable();
                sums.dedup();
                for noble in 0..self.kinds.len() {
                    for &distance in &sums {
                        let mut ns = st.clone();
                        ns.used.iter_mut().for_each(|u| *u = true);
                        ns.moved.push(noble);
                        self.shift(&mut ns, noble, distance as i32, true, board);
                        out.push((Action::MoveNobleCombined { noble, distance }, ns));
                    }
                }
            }
            return out;
        }
        if st.medals > 0 && st.medal_moves < MAX_MEDAL_MOVES {
            let max = self.max_move;
            for noble in 0..self.kinds.len() {
                for delta in (-max..=max).filter(|d| *d != 0) {
                    let from = st.pos[noble];
                    let to = (from as i32 + delta as i32).rem_euclid(board) as u8;
                    if !self.relevant(from, to) {
                        continue;
                    }
                    let mut ns = st.clone();
                    ns.pos[noble] = to;
                    ns.medals -= 1;
                    ns.supply += 1;
                    ns.medal_moves += 1;
                    out.push((Action::MedalMoveNoble { noble, delta }, ns));
                }
            }
        }
        out
    }

    /// Does moving between these squares change any arena membership that
    /// matters (mine, or a rival's)?
    fn relevant(&self, from: u8, to: u8) -> bool {
        self.arenas.iter().enumerate().any(|(i, a)| {
            let interesting = i == self.me.0 || self.opps.iter().any(|o| o.idx == i && (o.later || self.final_round));
            interesting && a.contains(from, self.board) != a.contains(to, self.board)
        })
    }

    fn shift(&self, st: &mut State, noble: usize, distance: i32, earns: bool, board: i32) {
        let to = (st.pos[noble] as i32 + distance).rem_euclid(board) as u8;
        st.pos[noble] = to;
        if earns && self.rest[to as usize] && st.supply > 0 {
            st.supply -= 1;
            st.medals += 1;
        }
    }

    fn search(&mut self, st: &State) -> (f64, Option<Action>) {
        let dice_left = st.used.iter().any(|u| !u);
        let mut best: (f64, Option<Action>) = (f64::NEG_INFINITY, None);
        if !dice_left {
            let (v, a) = self.leaf(st);
            best = (v, Some(a));
        }
        if self.nodes > NODE_BUDGET && !dice_left {
            return best;
        }
        for (act, ns) in self.successors(st) {
            let (v, _) = self.search(&ns);
            if v > best.0 {
                best = (v, Some(act));
            }
        }
        best
    }
}

/// Picks the best producing-phase action, or `None` to fall back.
pub fn plan(game: &Game, legal: &[Action]) -> Option<Action> {
    let me = game.current_player()?;
    let colosseum_engine::Phase::Producing(pr) = game.phase() else {
        return None;
    };
    let rules = game.rules();
    let n = game.num_players();
    let board = rules.board.size;
    let final_round = game.round() >= rules.num_rounds;
    let nobles = game.nobles();
    let kinds: Vec<NobleKind> = nobles.iter().map(|x| x.kind).collect();
    let values: Vec<u32> = kinds.iter().map(|&k| rules.noble_value(k)).collect();
    let arenas: Vec<Arena> = game.players().iter().map(|p| p.arena).collect();
    let rest: Vec<bool> = (0..board).map(|s| rules.is_resting_place(s)).collect();

    let mut progs = Vec::new();
    for p in &game.player(me).programs {
        if let Some(pv) = game.event_preview(me, p.id, 0) {
            progs.push((p.id, pv.breakdown.total() - pv.breakdown.nobles, !p.produced));
        }
    }

    let mut floor = 0;
    let mut opps = Vec::new();
    for o in game.players() {
        if o.id == me {
            continue;
        }
        let turn = (o.id.0 + n - game.start_player().0) % n;
        let later = turn > pr.turn;
        let mut base = 0;
        for p in &o.programs {
            if let Some(pv) = game.event_preview(o.id, p.id, 0) {
                base = base.max(pv.breakdown.total() - pv.breakdown.nobles + o.medals * rules.scoring.medal_bonus);
            }
        }
        if final_round {
            floor = floor.max(o.best_score);
        }
        // Weight rivals by how close to the lead they are.
        let lead = game.players().iter().map(|q| q.best_score).max().unwrap_or(0);
        let weight = if o.best_score + 8 >= lead { 0.55 } else { 0.3 };
        opps.push(Opp { idx: o.id.0, later, base, weight });
    }

    let mut ctx = Ctx {
        me,
        board,
        final_round,
        kinds,
        values,
        arenas,
        opps,
        progs,
        dice: pr.dice.clone(),
        rest,
        medal_bonus: rules.scoring.medal_bonus,
        max_move: rules.medals.max_noble_move as i8,
        floor,
        nodes: 0,
    };
    let root = State {
        pos: nobles.iter().map(|x| x.position).collect(),
        used: pr.used.clone(),
        moved: pr.moved.clone(),
        medals: game.player(me).medals,
        supply: game.supply().medals,
        medal_moves: 0,
        can_combine: pr.dice.len() == 2 && pr.moved.is_empty(),
    };
    let (_, act) = ctx.search(&root);
    let act = act?;
    if legal.contains(&act) {
        return Some(act);
    }
    // Skip is always the fall-back when nothing is producible.
    if legal.contains(&Action::SkipEvent) && matches!(act, Action::SkipEvent) {
        return Some(act);
    }
    None
}
