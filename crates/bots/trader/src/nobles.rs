//! A light model of noble movement: how many spectators the nobles in my
//! arena add, and the medals earned by landing on resting places. It works on
//! plain arrays so the dice can be searched without cloning the game.

use colosseum_engine::{Action, Game, PlayerId};

pub struct NobleSim<'a> {
    game: &'a Game,
    arena_start: u8,
    arena_len: u8,
    board: u8,
    values: Vec<f64>,
    medal_value: f64,
}

#[derive(Clone)]
struct State {
    pos: Vec<u8>,
    used: Vec<bool>,
    moved: Vec<usize>,
    supply: u32,
}

impl<'a> NobleSim<'a> {
    pub fn new(game: &'a Game, me: PlayerId, medal_value: f64) -> Self {
        let arena = game.player(me).arena;
        let rules = game.rules();
        Self {
            game,
            arena_start: arena.start,
            arena_len: arena.len,
            board: rules.board.size,
            values: game.nobles().iter().map(|n| rules.noble_value(n.kind) as f64).collect(),
            medal_value,
        }
    }

    fn in_arena(&self, square: u8) -> bool {
        let offset = (square as u32 + self.board as u32 - self.arena_start as u32) % self.board as u32;
        offset < self.arena_len as u32
    }

    fn arena_value(&self, pos: &[u8]) -> f64 {
        pos.iter()
            .zip(&self.values)
            .filter(|(p, _)| self.in_arena(**p))
            .map(|(_, v)| *v)
            .sum()
    }

    fn shift(&self, from: u8, distance: i32) -> u8 {
        (from as i32 + distance).rem_euclid(self.board as i32) as u8
    }

    /// Moves one noble; returns the medal value earned.
    fn step(&self, st: &mut State, noble: usize, distance: i32, earns: bool) -> f64 {
        let to = self.shift(st.pos[noble], distance);
        st.pos[noble] = to;
        if earns && st.supply > 0 && self.game.rules().is_resting_place(to) {
            st.supply -= 1;
            self.medal_value
        } else {
            0.0
        }
    }

    /// Best total (arena value plus medals) reachable by spending the rest of
    /// the dice, with the action that starts it when `first` is wanted.
    fn best(&self, st: &State, dice: &[Vec<u8>], combined: &[u8]) -> f64 {
        let mut best = f64::NEG_INFINITY;
        for die in (0..dice.len()).filter(|d| !st.used[*d]) {
            for noble in (0..st.pos.len()).filter(|n| !st.moved.contains(n)) {
                for &d in &dice[die] {
                    let mut next = st.clone();
                    next.used[die] = true;
                    next.moved.push(noble);
                    let earned = self.step(&mut next, noble, d as i32, true);
                    best = best.max(earned + self.best(&next, dice, combined));
                }
            }
        }
        if st.moved.is_empty() && dice.len() == 2 && st.used.iter().all(|u| !u) {
            for noble in 0..st.pos.len() {
                for &d in combined {
                    let mut next = st.clone();
                    next.used.iter_mut().for_each(|u| *u = true);
                    next.moved.push(noble);
                    let earned = self.step(&mut next, noble, d as i32, true);
                    best = best.max(earned + self.arena_value(&next.pos));
                }
            }
        }
        if best == f64::NEG_INFINITY {
            self.arena_value(&st.pos)
        } else {
            best
        }
    }

    /// Value of each dice move in `legal`, looking ahead through the other die.
    pub fn score_dice_moves(&self, legal: &[Action]) -> Vec<(f64, Action)> {
        let Some(pr) = (match self.game.phase() {
            colosseum_engine::Phase::Producing(pr) => Some(pr),
            _ => None,
        }) else {
            return Vec::new();
        };
        let dice: Vec<Vec<u8>> = pr.dice.iter().map(|f| f.values().to_vec()).collect();
        let combined = pr.combined_distances();
        let base = State {
            pos: self.game.nobles().iter().map(|n| n.position).collect(),
            used: pr.used.clone(),
            moved: pr.moved.clone(),
            supply: self.game.supply().medals,
        };
        let mut out = Vec::new();
        for a in legal {
            let mut st = base.clone();
            let v = match a {
                Action::MoveNoble { noble, die, distance } => {
                    st.used[*die] = true;
                    st.moved.push(*noble);
                    let earned = self.step(&mut st, *noble, *distance as i32, true);
                    earned + self.best(&st, &dice, &combined)
                }
                Action::MoveNobleCombined { noble, distance } => {
                    st.used.iter_mut().for_each(|u| *u = true);
                    st.moved.push(*noble);
                    let earned = self.step(&mut st, *noble, *distance as i32, true);
                    earned + self.arena_value(&st.pos)
                }
                _ => continue,
            };
            out.push((v, a.clone()));
        }
        out
    }

    /// The best medal move worth its medal, if any, with its net gain.
    pub fn best_medal_move(&self, legal: &[Action]) -> Option<(f64, Action)> {
        let pos: Vec<u8> = self.game.nobles().iter().map(|n| n.position).collect();
        let now = self.arena_value(&pos);
        let mut best: Option<(f64, Action)> = None;
        for a in legal {
            if let Action::MedalMoveNoble { noble, delta } = a {
                let mut p = pos.clone();
                p[*noble] = self.shift(p[*noble], *delta as i32);
                let gain = self.arena_value(&p) - now - self.medal_value;
                if gain > 0.01 && best.as_ref().is_none_or(|b| gain > b.0) {
                    best = Some((gain, a.clone()));
                }
            }
        }
        best
    }
}
