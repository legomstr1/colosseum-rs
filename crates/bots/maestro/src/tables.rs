//! Game data flattened into plain arrays once per agent, so the valuation
//! never has to search the config or allocate.

use colosseum_engine::{Asset, Game};

pub const NA: usize = Asset::COUNT;
pub const JOKER: usize = Asset::Joker as usize;
pub const EMPEROR: usize = Asset::Emperor as usize;
pub const SPECIAL: usize = Asset::SpecialEvent as usize;

/// One event card.
#[derive(Clone, Debug)]
pub struct Ev {
    pub req: [u8; NA],
    /// Required asset types (indices), each once.
    pub types: Vec<usize>,
    /// `score[k]` is the base score with `k` assets missing.
    pub score: Vec<f64>,
    pub cost: u32,
    pub squares: u8,
}

impl Ev {
    pub fn max_missing(&self) -> usize {
        self.score.len() - 1
    }

    /// Base score with a fractional number of missing assets; beyond the
    /// card's limit it keeps falling so the gradient never vanishes.
    pub fn interp(&self, missing: f64) -> f64 {
        let s = |k: usize| -> f64 {
            if k < self.score.len() {
                self.score[k]
            } else {
                let last = self.score[self.score.len() - 1];
                (last - 6.0 * (k - self.max_missing()) as f64).max(0.0)
            }
        };
        let m = missing.max(0.0);
        let lo = m.floor() as usize;
        let f = m - lo as f64;
        s(lo) * (1.0 - f) + s(lo + 1) * f
    }

    pub fn missing(&self, a: &[u8; NA]) -> u32 {
        let mut m = 0u32;
        for &t in &self.types {
            m += self.req[t].saturating_sub(a[t]) as u32;
        }
        m.saturating_sub(a[JOKER] as u32)
    }
}

pub struct Tables {
    /// Indexed by program id.
    pub ev: Vec<Option<Ev>>,
    pub rounds: u32,
    pub board: u8,
    pub resting: Vec<bool>,
    /// Spectators of each noble, by noble index.
    pub noble_val: Vec<f64>,
    pub star: [bool; NA],
    pub star_thr: u8,
    pub s_prev: f64,
    pub s_ticket: f64,
    pub s_star: f64,
    pub s_podium: f64,
    pub s_medal: f64,
    pub ticket_cost: u32,
    pub exp_cost: u32,
    pub loge_cost: u32,
    pub max_squares: u8,
    pub min_bid: u32,
    pub extra_medal_cost: u32,
    pub tradeable: [bool; NA],
}

impl Tables {
    pub fn new(game: &Game) -> Tables {
        let cfg = game.config();
        let r = &cfg.rules;
        let max_id = cfg.events.iter().map(|e| e.id as usize).max().unwrap_or(0);
        let mut ev: Vec<Option<Ev>> = vec![None; max_id + 1];
        for e in &cfg.events {
            let mut req = [0u8; NA];
            for a in &e.requirements {
                req[a.index()] += 1;
            }
            let types = (0..NA).filter(|&i| req[i] > 0).collect();
            let mut score = vec![e.base_score as f64];
            score.extend(e.penalty_scores.iter().map(|&s| s as f64));
            let squares = r
                .arena
                .squares_required_by_event_size
                .get(e.size as usize)
                .copied()
                .unwrap_or(u8::MAX);
            ev[e.id as usize] = Some(Ev {
                req,
                types,
                score,
                cost: e.cost,
                squares,
            });
        }
        let board = r.board.size;
        let resting = (0..board).map(|sq| r.is_resting_place(sq)).collect();
        let noble_val = game.nobles().iter().map(|n| r.noble_value(n.kind) as f64).collect();
        let mut star = [false; NA];
        for a in &r.stars.assets {
            star[a.index()] = true;
        }
        let mut tradeable = [false; NA];
        for a in Asset::ALL {
            tradeable[a.index()] = a.is_tradeable();
        }
        Tables {
            ev,
            rounds: r.num_rounds,
            board,
            resting,
            noble_val,
            star,
            star_thr: r.stars.threshold as u8,
            s_prev: r.scoring.previous_program_bonus as f64,
            s_ticket: r.scoring.season_ticket_bonus as f64,
            s_star: r.scoring.star_award_bonus as f64,
            s_podium: r.scoring.podium_bonus as f64,
            s_medal: r.scoring.medal_bonus as f64,
            ticket_cost: r.investing.season_ticket_cost,
            exp_cost: r.arena.expansion_cost,
            loge_cost: r.investing.emperor_loge_cost,
            max_squares: r.arena.max_squares,
            min_bid: r.acquiring.minimum_opening_bid,
            extra_medal_cost: r.investing.extra_investment_medal_cost,
            tradeable,
        }
    }

    pub fn ev(&self, id: u8) -> Option<&Ev> {
        self.ev.get(id as usize).and_then(|e| e.as_ref())
    }

    pub fn in_arena(&self, start: u8, len: u8, sq: u8) -> bool {
        let off = (sq as u32 + self.board as u32 - start as u32) % self.board as u32;
        off < len as u32
    }
}
