//! The valuation model: what a holding is worth, in points of projected
//! final-event score, for any player. One model prices everything (bids,
//! investments, trades, discards) for me and, for denial and trade
//! judgement, for my opponents.

use crate::Params;
use crate::tables::{EMPEROR, JOKER, NA, SPECIAL, Tables};
use colosseum_engine::{Game, Phase, PlayerState};

pub const MAXP: usize = 8;
const MAXPROG: usize = 12;

/// A player's holdings, as plain copyable data for what-if questions.
#[derive(Clone, Copy, Debug)]
pub struct Hold {
    pub who: usize,
    pub a: [u8; NA],
    pub coins: i32,
    pub progs: [u8; MAXPROG],
    /// Bit `i` set when `progs[i]` has been produced.
    pub produced: u16,
    pub np: usize,
    pub arena: u8,
    pub tickets: u32,
    pub loge: bool,
    pub medals: u32,
    pub podiums: u32,
}

impl Hold {
    pub fn of(p: &PlayerState) -> Hold {
        let mut a = [0u8; NA];
        for (asset, n) in p.assets.iter() {
            a[asset.index()] = n.min(255) as u8;
        }
        let mut progs = [0u8; MAXPROG];
        let mut produced = 0u16;
        let np = p.programs.len().min(MAXPROG);
        for (i, q) in p.programs.iter().take(MAXPROG).enumerate() {
            progs[i] = q.id;
            if q.produced {
                produced |= 1 << i;
            }
        }
        Hold {
            who: p.id.0,
            a,
            coins: p.coins as i32,
            progs,
            produced,
            np,
            arena: p.arena.len,
            tickets: p.season_tickets,
            loge: p.loge,
            medals: p.medals,
            podiums: p.podiums,
        }
    }

    pub fn add_program(&mut self, id: u8) {
        if self.np < MAXPROG {
            self.progs[self.np] = id;
            self.np += 1;
        }
    }

    /// Adds tokens as won at auction: Emperor tokens become medals while
    /// the supply lasts; SpecialEvent tokens are kept.
    pub fn gain_tokens(&mut self, tokens: &[colosseum_engine::Asset], medal_supply: &mut u32) {
        for &t in tokens {
            let i = t.index();
            if i == EMPEROR {
                if *medal_supply > 0 {
                    *medal_supply -= 1;
                    self.medals += 1;
                }
            } else {
                self.a[i] = self.a[i].saturating_add(1);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Invest,
    Acquire,
    Trade,
    Produce,
    Close,
    Over,
}

/// Per-decision context shared by every valuation.
#[derive(Clone)]
pub struct Ctx<'a> {
    pub g: &'a Game,
    pub t: &'a Tables,
    pub p: &'a Params,
    pub n: usize,
    pub round: u32,
    pub last: bool,
    pub stage: Stage,
    /// Asset counts per player (overridable for trade what-ifs).
    pub counts: [[u8; NA]; MAXP],
    /// Star holder per asset, or 255.
    pub holder: [u8; NA],
    pub medal_supply: u32,
    /// Points per coin now.
    pub lam: f64,
    /// Investing turn index (for slot counting).
    inv_turn: usize,
    inv_remaining: u32,
    prod_turn: usize,
    acq_won: [bool; MAXP],
}

/// How much of the game is left for one player.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub slots: u32,
    pub acq: f64,
    pub prods_bf: u32,
    pub prods_incl_final: u32,
}

impl<'a> Ctx<'a> {
    pub fn new(g: &'a Game, t: &'a Tables, p: &'a Params) -> Ctx<'a> {
        let n = g.num_players().min(MAXP);
        let mut counts = [[0u8; NA]; MAXP];
        for (i, pl) in g.players().iter().take(MAXP).enumerate() {
            for (asset, c) in pl.assets.iter() {
                counts[i][asset.index()] = c.min(255) as u8;
            }
        }
        let mut holder = [255u8; NA];
        for (a, h) in g.stars() {
            if let Some(h) = h {
                holder[a.index()] = h.0 as u8;
            }
        }
        let (stage, inv_turn, inv_remaining, prod_turn) = match g.phase() {
            Phase::Investing { turn, remaining } => (Stage::Invest, *turn, *remaining, 0),
            Phase::Acquiring(_) => (Stage::Acquire, 0, 0, 0),
            Phase::Trading { .. } => (Stage::Trade, 0, 0, 0),
            Phase::Producing(pr) => (Stage::Produce, 0, 0, pr.turn),
            Phase::Closing(_) => (Stage::Close, 0, 0, 0),
            Phase::GameOver => (Stage::Over, 0, 0, 0),
        };
        let mut acq_won = [false; MAXP];
        if let Phase::Acquiring(a) = g.phase() {
            for (i, w) in a.won.iter().take(MAXP).enumerate() {
                acq_won[i] = *w;
            }
        }
        let round = g.round();
        let last = round >= t.rounds;
        let lam = Self::lambda(p, t, round, stage);
        Ctx {
            g,
            t,
            p,
            n,
            round,
            last,
            stage,
            counts,
            holder,
            medal_supply: g.supply().medals,
            lam,
            inv_turn,
            inv_remaining,
            prod_turn,
            acq_won,
        }
    }

    /// Points per coin, falling as the chances to spend coins run out.
    fn lambda(p: &Params, t: &Tables, round: u32, stage: Stage) -> f64 {
        let rounds = t.rounds.max(1);
        if round >= rounds {
            return match stage {
                Stage::Invest => p.lam_last_inv,
                Stage::Acquire => p.lam_last_acq,
                Stage::Trade => p.lam_last_trade,
                _ => p.lam_end,
            };
        }
        let left = (rounds - round) as f64; // full rounds after this one
        let frac = left / (rounds - 1).max(1) as f64; // 1 in round 1 .. ~0 in the last
        let base = p.lam_late + (p.lam_early - p.lam_late) * frac;
        match stage {
            Stage::Produce | Stage::Close => {
                // Coins earned now are first spendable next round.
                let next = (rounds - round - 1) as f64 / (rounds - 1).max(1) as f64;
                p.lam_late + (p.lam_early - p.lam_late) * next
            }
            _ => base,
        }
    }

    /// Seat order index of `who` counting from the start player.
    pub fn seat_index(&self, who: usize) -> usize {
        (who + self.n - self.g.start_player().0) % self.n
    }

    pub fn timing(&self, who: usize) -> Timing {
        let rounds = self.t.rounds;
        let later = rounds.saturating_sub(self.round);
        let si = self.seat_index(who);
        let (slots, acq, prods_bf) = match self.stage {
            Stage::Invest => {
                let now = if si < self.inv_turn {
                    0
                } else if si == self.inv_turn {
                    self.inv_remaining
                } else {
                    1
                };
                (now + later, (later + 1) as f64, later)
            }
            Stage::Acquire => {
                let now = if self.acq_won[who.min(MAXP - 1)] { 0.0 } else { self.p.acq_now };
                (later, later as f64 + now, later)
            }
            Stage::Trade => (later, later as f64, later),
            Stage::Produce => {
                let done = si < self.prod_turn;
                (later, later as f64, if done { later.saturating_sub(1) } else { later })
            }
            Stage::Close | Stage::Over => (later, later as f64, later.saturating_sub(1)),
        };
        let prods_bf = if self.last { 0 } else { prods_bf };
        let prods_incl_final = if self.stage == Stage::Over {
            0
        } else {
            prods_bf + 1
        };
        Timing {
            slots,
            acq,
            prods_bf,
            prods_incl_final,
        }
    }

    /// Whether `who` would hold the star for asset `a` with `cnt` of it.
    pub fn star_held(&self, who: usize, a: usize, cnt: u8) -> bool {
        if !self.t.star[a] || cnt < self.t.star_thr {
            return false;
        }
        let mut om = 0u8;
        for q in 0..self.n {
            if q != who {
                om = om.max(self.counts[q][a]);
            }
        }
        cnt > om || (cnt == om && self.holder[a] as usize == who)
    }

    /// Projected final score with `id` as the final event, excluding the
    /// terms every target shares. `None` if it can't be ready in time.
    pub fn target(&self, id: u8, h: &Hold, tm: &Timing) -> Option<f64> {
        let ev = self.t.ev(id)?;
        let p = self.p;
        let d = ev.squares.saturating_sub(h.arena) as u32;
        if d > tm.slots || ev.squares > self.t.max_squares {
            return None;
        }
        let exp = (d * self.t.exp_cost) as f64;
        let budget = h.coins as f64 + p.income * tm.prods_bf as f64 - exp;
        if budget < 0.0 && d > 0 {
            return None;
        }
        let missing = ev.missing(&h.a) as f64;
        let gain = (p.gain_rate * tm.acq).min(budget.max(0.0) / p.token_cost);
        let base = ev.interp(missing - gain);

        // Other programs: produced ones count; unproduced ones count if
        // there are productions left for them and they can be staged.
        let mut po = 0u32;
        let mut pu = 0u32;
        for i in 0..h.np {
            if h.progs[i] == id {
                continue;
            }
            if h.produced & (1 << i) != 0 {
                po += 1;
            } else if let Some(o) = self.t.ev(h.progs[i])
                && o.squares <= h.arena.max(ev.squares)
                && (o.missing(&h.a) as f64 - p.gain_rate * tm.acq * 0.5) <= o.max_missing() as f64
            {
                pu += 1;
            }
        }
        let prev = po as f64 + p.prev_disc * pu.min(tm.prods_bf) as f64;

        let mut stars = 0.0;
        for &a in &ev.types {
            if self.star_held(h.who, a, h.a[a]) {
                stars += 1.0;
            }
        }
        Some(
            base + prev * self.t.s_prev + stars * self.t.s_star * p.star_w
                - d as f64 * (self.t.exp_cost as f64 * self.lam + p.slot_pen),
        )
    }

    /// Worth of a holding, in points.
    pub fn value(&self, h: &Hold) -> f64 {
        let tm = self.timing(h.who);
        self.value_t(h, &tm)
    }

    pub fn value_t(&self, h: &Hold, tm: &Timing) -> f64 {
        let p = self.p;
        let mut best = f64::NEG_INFINITY;
        let mut second = f64::NEG_INFINITY;
        for i in 0..h.np {
            if let Some(s) = self.target(h.progs[i], h, tm) {
                if s > best {
                    second = best;
                    best = s;
                } else if s > second {
                    second = s;
                }
            }
        }
        if best == f64::NEG_INFINITY {
            best = 0.0;
        }
        let second = if second == f64::NEG_INFINITY { 0.0 } else { second };
        let t = self.t;
        let loge = if h.loge {
            p.loge_round * tm.prods_incl_final as f64 * self.medal_factor()
        } else {
            0.0
        };
        let nobles = p.nob_sq * h.arena as f64;
        let special = h.a[SPECIAL] as f64 * self.special_val();
        let jokers = h.a[JOKER] as f64 * p.joker_extra;
        best + p.second_w * second.max(0.0)
            + h.tickets as f64 * t.s_ticket
            + h.podiums as f64 * t.s_podium
            + h.medals as f64 * p.medal_val
            + loge
            + nobles
            + special
            + jokers
            + h.coins.max(0) as f64 * self.lam
    }

    /// How much of the dice's medal income is still available.
    pub fn medal_factor(&self) -> f64 {
        (self.medal_supply as f64 / (self.n as f64 * 1.5)).min(1.0)
    }

    pub fn special_val(&self) -> f64 {
        if self.medal_supply > 0 {
            self.p.special_val
        } else {
            self.p.special_val * 0.4
        }
    }

    /// Change in `who`'s worth from gaining `lot` (as at auction).
    pub fn lot_gain(&self, who: usize, lot: &[colosseum_engine::Asset]) -> f64 {
        let h = Hold::of(&self.g.players()[who]);
        let tm = self.timing(who);
        let before = self.value_t(&h, &tm);
        let mut h2 = h;
        let mut supply = self.medal_supply;
        h2.gain_tokens(lot, &mut supply);
        self.value_t(&h2, &tm) - before
    }
}
