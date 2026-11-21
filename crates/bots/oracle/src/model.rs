//! The oracle's valuation model.
//!
//! Every holding is valued in points of projected final-event score (the
//! number that wins the game) plus a coin term. All rule numbers come from
//! `game.rules()` / `game.config()`; the oracle's own tuning lives in
//! [`Params`]. Holdings are plain `Copy` arrays so what-if questions never
//! allocate.

use colosseum_engine::{Asset, Game, Phase, PlayerId, PlayerState};

pub const NA: usize = Asset::COUNT;
pub const MAXP: usize = 5;
const MAXPROG: usize = 16;

/// Tuning knobs (heuristics, not rules).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Params {
    /// Useful tokens expected from each future acquiring phase.
    pub gain_rate: f64,
    /// Useful tokens expected from each future trading phase.
    pub trade_rate: f64,
    /// Coins one useful token costs.
    pub token_cost: f64,
    /// Coins expected from each production before the final one.
    pub income: f64,
    /// Points per coin, by round (index 0 = round 1); the last entry is used
    /// for later rounds.
    pub lambda: Vec<f64>,
    /// Points per coin in the final round once acquiring is over.
    pub lambda_end: f64,
    /// Points per coin in the final round's acquiring phase.
    pub lambda_final_acq: f64,
    /// Discount on tickets the plan has not bought yet.
    pub ticket_disc: f64,
    /// Expected noble spectators in the final event: base, per extra arena
    /// square, and for owning a loge.
    pub noble_base: f64,
    pub noble_sq: f64,
    pub noble_loge: f64,
    /// Value of a loge per remaining production (medals, control).
    pub loge_round: f64,
    /// Weight of the second-best program.
    pub second: f64,
    /// Value of a SpecialEvent token.
    pub se_value: f64,
    /// Extra value per point of a non-final event above my best so far.
    pub best_w: f64,
    /// Medal value in points (non-final rounds).
    pub medal_value: f64,
    /// Smallest gain that justifies an investment.
    pub invest_min: f64,
    /// Weight of an opponent's gain against mine in trades.
    pub trade_k: f64,
    /// Smallest relative gain to accept an offer.
    pub accept_margin: f64,
    /// Smallest gain from a token worth trying to buy.
    pub buy_min: f64,
    /// Most offers I make per trading turn.
    pub max_offers: u32,
    /// Share of the trade surplus I'm willing to pay as coins.
    pub pay_frac: f64,
    /// Weight of denying an opponent a lot.
    pub deny: f64,
    /// Share of the best alternative lot's surplus deducted from a lot.
    pub alt_frac: f64,
    /// Share of a lot's value I bid up to.
    pub bid_frac: f64,
    /// Win-probability temperature in the final round (points).
    pub tau: f64,
    /// Expected extra points an opponent gets from dice and medals in the
    /// final round on top of what they can produce now.
    pub opp_extra: f64,
    /// Penalty for each arena square the plan still needs.
    pub step_penalty: f64,
    /// Weight of the coin income from tickets and podiums.
    pub income_w: f64,
    /// Flat value of each standard token and each Joker held.
    pub token_w: f64,
    pub joker_w: f64,
    /// Value of winning a podium (points), and its probability temperature.
    pub podium_w: f64,
    pub podium_tau: f64,
    /// Open an auction when the lot is worth this share of the minimum bid.
    pub open_frac: f64,
    /// Coins kept in reserve for tickets and bids before the final round are
    /// worth `reserve_mult` times a normal coin.
    pub reserve: f64,
    pub reserve_mult: f64,
    /// Pick the lot to open by expected surplus against rival ceilings (1)
    /// or by raw worth (0).
    pub smart_open: f64,
    /// Offers to a partner who has never accepted before I only probe them.
    pub probe_tries: u32,
    /// Start the price ladder at a partner's lowest accepted price.
    pub use_floor: f64,
    /// Weight of token swaps among offers (0 turns them off), and the
    /// smallest worth of my token to the partner for a swap.
    pub swaps: f64,
    pub swap_min: f64,
    /// Temperature (points) of the rival threat weights; 0 turns them off.
    pub threat_tau: f64,
    /// Weight of the win chance (against my own score) in final-round dice.
    pub final_wp: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            gain_rate: 0.4,
            trade_rate: 0.061,
            token_cost: 5.0,
            income: 10.0,
            lambda: vec![0.16, 0.1365, 0.108, 0.08, 0.04],
            lambda_end: 0.02,
            lambda_final_acq: 0.1,
            ticket_disc: 0.3,
            noble_base: 4.0,
            noble_sq: 1.5,
            noble_loge: 2.0,
            loge_round: 3.0,
            second: 0.1,
            se_value: 3.0,
            best_w: 0.1,
            medal_value: 3.0,
            invest_min: 0.3,
            trade_k: 0.8,
            accept_margin: 0.5,
            buy_min: 0.5,
            max_offers: 30,
            pay_frac: 0.7,
            deny: 0.1,
            alt_frac: 0.9,
            bid_frac: 0.9,
            tau: 4.0,
            opp_extra: 3.0,
            step_penalty: 4.0,
            income_w: 2.0,
            token_w: 0.768,
            joker_w: 1.0,
            podium_w: 4.0,
            podium_tau: 3.0,
            open_frac: 0.667,
            reserve: 20.0,
            reserve_mult: 2.5,
            smart_open: 1.0,
            probe_tries: 12,
            use_floor: 1.0,
            swaps: 0.5,
            swap_min: 1.0,
            threat_tau: 12.0,
            final_wp: 1.0,
        }
    }
}

/// One event card, in array form.
#[derive(Clone, Debug)]
pub struct Prog {
    pub id: u8,
    pub cost: u32,
    pub base: u32,
    pub squares: u8,
    pub req: [u32; NA],
    pub penalties: Vec<u32>,
}

impl Prog {
    pub fn score_int(&self, missing: u32) -> Option<u32> {
        if missing == 0 {
            Some(self.base)
        } else {
            self.penalties.get(missing as usize - 1).copied()
        }
    }

    fn score_at(&self, k: usize) -> f64 {
        if k == 0 {
            return self.base as f64;
        }
        match self.penalties.get(k - 1) {
            Some(v) => *v as f64,
            None => {
                let last = self.penalties.last().copied().unwrap_or(self.base) as f64;
                (last - 6.0 * (k - self.penalties.len()) as f64).max(0.0)
            }
        }
    }

    /// Score with a fractional number of missing assets, interpolated.
    pub fn score_frac(&self, m: f64) -> f64 {
        let m = m.max(0.0);
        let lo = m.floor() as usize;
        let f = m - lo as f64;
        self.score_at(lo) * (1.0 - f) + self.score_at(lo + 1) * f
    }

    /// Missing assets after Jokers.
    pub fn missing(&self, a: &[u32; NA]) -> u32 {
        let mut m = 0;
        for (i, &r) in self.req.iter().enumerate() {
            m += r.saturating_sub(a[i]);
        }
        m.saturating_sub(a[Asset::Joker.index()])
    }
}

/// Static tables built once from the config.
pub struct Tables {
    pub progs: Vec<Option<Prog>>,
    pub star: [bool; NA],
    pub resting: Vec<bool>,
    pub noble_vals: Vec<u32>,
}

impl Tables {
    pub fn new(game: &Game) -> Self {
        let rules = game.rules();
        let cfg = game.config();
        let max_id = cfg.events.iter().map(|e| e.id as usize).max().unwrap_or(0);
        let mut progs = vec![None; max_id + 1];
        for e in &cfg.events {
            let mut req = [0u32; NA];
            for a in &e.requirements {
                req[a.index()] += 1;
            }
            let squares = rules
                .arena
                .squares_required_by_event_size
                .get(e.size as usize)
                .copied()
                .unwrap_or(u8::MAX);
            progs[e.id as usize] = Some(Prog {
                id: e.id,
                cost: e.cost,
                base: e.base_score,
                squares,
                req,
                penalties: e.penalty_scores.clone(),
            });
        }
        let mut star = [false; NA];
        for a in &rules.stars.assets {
            star[a.index()] = true;
        }
        let size = rules.board.size as usize;
        let mut resting = vec![false; size.max(1)];
        for (sq, _) in &rules.noble_positions {
            if (*sq as usize) < resting.len() {
                resting[*sq as usize] = true;
            }
        }
        let noble_vals = game.nobles().iter().map(|n| rules.noble_value(n.kind)).collect();
        Tables {
            progs,
            star,
            resting,
            noble_vals,
        }
    }

    pub fn prog(&self, id: u8) -> Option<&Prog> {
        self.progs.get(id as usize).and_then(|p| p.as_ref())
    }
}

/// A player's holding, editable for what-if questions.
#[derive(Clone, Copy, Debug)]
pub struct Hyp {
    pub a: [u32; NA],
    pub coins: u32,
    pub progs: [u8; MAXPROG],
    pub produced: u32,
    pub np: usize,
    pub arena: u8,
    pub tickets: u32,
    pub loge: bool,
    pub medals: u32,
    pub podiums: u32,
}

impl Hyp {
    pub fn of(p: &PlayerState) -> Hyp {
        let mut a = [0u32; NA];
        for (x, n) in p.assets.iter() {
            a[x.index()] = n;
        }
        let mut progs = [0u8; MAXPROG];
        let mut produced = 0u32;
        let mut np = 0;
        for q in p.programs.iter().take(MAXPROG) {
            progs[np] = q.id;
            if q.produced {
                produced |= 1 << np;
            }
            np += 1;
        }
        Hyp {
            a,
            coins: p.coins,
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

    pub fn add_prog(&mut self, id: u8) {
        if self.np < MAXPROG {
            self.progs[self.np] = id;
            self.np += 1;
        }
    }

    pub fn set_produced(&mut self, id: u8) {
        for i in 0..self.np {
            if self.progs[i] == id {
                self.produced |= 1 << i;
            }
        }
    }


    /// Adds won tokens; Emperor tokens become medals while the supply lasts.
    pub fn add_tokens(&mut self, tokens: &[Asset], medal_supply: u32) {
        let mut supply = medal_supply;
        for &t in tokens {
            if t == Asset::Emperor {
                if supply > 0 {
                    supply -= 1;
                    self.medals += 1;
                }
            } else {
                self.a[t.index()] += 1;
            }
        }
    }
}

/// How much of the game is left, from one player's point of view.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Investments still to make (this round's if not yet made, and later).
    pub slots: u32,
    /// Acquiring phases still to come.
    pub acq_left: f64,
    /// Trading phases still to come.
    pub trade_left: f64,
    /// Productions still to come before the final round's.
    pub prods_before_final: u32,
    /// Productions still to come, final included.
    pub prods_left: u32,
    /// Points per coin.
    pub lambda: f64,
    /// Coins worth `reserve_mult` times more.
    pub reserve: f64,
}

/// Everything a decision needs, computed once per decision.
pub struct Ctx<'a> {
    pub g: &'a Game,
    pub t: &'a Tables,
    pub p: &'a Params,
    pub me: PlayerId,
    pub n: usize,
    pub round: u32,
    pub rounds: u32,
    pub last: bool,
    pub cnt: [[u32; NA]; MAXP],
    pub holder: [Option<usize>; NA],
    pub thr: u32,
    pub tickets_supply: u32,
    pub medal_supply: u32,
    /// How much each rival threatens me (1 = an even rival), used to weight
    /// denial and trade partners' gains.
    pub threat: [f64; MAXP],
}

/// Phase order index: investing 0 .. closing 4.
pub fn phase_index(ph: &Phase) -> u32 {
    match ph {
        Phase::Investing { .. } => 0,
        Phase::Acquiring(_) => 1,
        Phase::Trading { .. } => 2,
        Phase::Producing(_) => 3,
        Phase::Closing(_) => 4,
        Phase::GameOver => 5,
    }
}

impl<'a> Ctx<'a> {
    pub fn new(g: &'a Game, t: &'a Tables, p: &'a Params, me: PlayerId) -> Self {
        let n = g.num_players().min(MAXP);
        let mut cnt = [[0u32; NA]; MAXP];
        for (i, pl) in g.players().iter().take(MAXP).enumerate() {
            for (x, c) in pl.assets.iter() {
                cnt[i][x.index()] = c;
            }
        }
        let mut holder = [None; NA];
        for (a, h) in g.stars() {
            holder[a.index()] = h.map(|x| x.0);
        }
        let rules = g.rules();
        let round = g.round();
        let rounds = rules.num_rounds;
        Ctx {
            g,
            t,
            p,
            me,
            n,
            round,
            rounds,
            last: round >= rounds,
            cnt,
            holder,
            thr: rules.stars.threshold,
            tickets_supply: g.supply().season_tickets,
            medal_supply: g.supply().medals,
            threat: [1.0; MAXP],
        }
    }

    /// Fills `threat` from each player's projected worth against mine.
    pub fn with_threats(mut self) -> Self {
        if self.p.threat_tau <= 0.0 {
            return self;
        }
        let tm = self.timing(0);
        let mut v = [0.0f64; MAXP];
        for (j, x) in v.iter_mut().enumerate().take(self.n) {
            *x = self.value(j, &self.hyp(j), &tm, None);
        }
        let me = self.me.0;
        for j in 0..self.n {
            if j != me {
                let z = (v[j] - v[me]) / self.p.threat_tau;
                self.threat[j] = 2.0 / (1.0 + (-z).exp());
            }
        }
        self
    }

    /// Points per coin at a point of the game.
    pub fn lambda_at(&self, phase_idx: u32) -> f64 {
        if self.last {
            if phase_idx <= 1 {
                return self.p.lambda_final_acq;
            }
            if phase_idx == 2 {
                return (self.p.lambda_final_acq + self.p.lambda_end) * 0.5;
            }
            return self.p.lambda_end;
        }
        let i = (self.round.saturating_sub(1)) as usize;
        let l = &self.p.lambda;
        l.get(i).or(l.last()).copied().unwrap_or(0.2)
    }

    /// Timing seen from the current phase. `inv_now` is the number of
    /// investments still available this round to the player.
    pub fn timing(&self, inv_now: u32) -> Timing {
        let ph = phase_index(self.g.phase());
        self.timing_at(ph, inv_now)
    }

    pub fn timing_at(&self, ph: u32, inv_now: u32) -> Timing {
        let later = self.rounds.saturating_sub(self.round);
        let slots = later + if ph == 0 { inv_now } else { 0 };
        let acq_left = (later + u32::from(ph <= 1)) as f64;
        let trade_left = (later + u32::from(ph <= 2)) as f64;
        let prods_before_final = if self.last {
            0
        } else {
            later - 1 + u32::from(ph <= 3)
        };
        let prods_left = later + u32::from(ph <= 3);
        Timing {
            slots,
            acq_left,
            trade_left,
            prods_before_final,
            prods_left,
            lambda: self.lambda_at(ph),
            reserve: if self.last { 0.0 } else { self.p.reserve },
        }
    }

    /// Whether `who` would hold `a`'s star with `c` of it, everyone else as now
    /// (or with `other` overriding one other player's count).
    pub fn holds_star(&self, who: usize, a: usize, c: u32, other: Option<(usize, u32)>) -> bool {
        if !self.t.star[a] || c < self.thr {
            return false;
        }
        let mut om = 0;
        for j in 0..self.n {
            if j == who {
                continue;
            }
            let v = match other {
                Some((o, oc)) if o == j => oc,
                _ => self.cnt[j][a],
            };
            om = om.max(v);
        }
        c > om || (c == om && self.holder[a] == Some(who))
    }

    fn stars_for(&self, who: usize, h: &Hyp, prog: &Prog, other: Option<(usize, &[u32; NA])>) -> u32 {
        let mut s = 0;
        for a in 0..NA {
            if prog.req[a] > 0 && self.t.star[a] {
                let o = other.map(|(j, arr)| (j, arr[a]));
                if self.holds_star(who, a, h.a[a], o) {
                    s += 1;
                }
            }
        }
        s
    }

    /// Projected final-event score of `prog` (net of planned spending in
    /// points), or `None` if it can't be ready in time.
    pub fn project(&self, who: usize, h: &Hyp, prog: &Prog, tm: &Timing, other: Option<(usize, &[u32; NA])>) -> Option<f64> {
        let rules = self.g.rules();
        let prm = self.p;
        let sq_need = prog.squares.saturating_sub(h.arena) as u32;
        if sq_need > tm.slots {
            return None;
        }
        let exp_cost = (sq_need * rules.arena.expansion_cost) as f64;
        let budget = h.coins as f64 + prm.income * tm.prods_before_final as f64;
        if exp_cost > budget {
            return None;
        }
        let missing = prog.missing(&h.a) as f64;
        let mut gain = prm.gain_rate * tm.acq_left + prm.trade_rate * tm.trade_left;
        let spare = budget - exp_cost;
        gain = gain.min(spare / prm.token_cost).max(0.0);
        let used_gain = gain.min(missing);
        let mf = missing - used_gain;
        let base = prog.score_frac(mf);
        let token_spend = used_gain * prm.token_cost;

        let s = &rules.scoring;
        let free = tm.slots - sq_need;
        let tcost = rules.investing.season_ticket_cost.max(1) as f64;
        let afford = ((spare - token_spend) / tcost).max(0.0);
        let planned = (free as f64).min(afford).min(self.tickets_supply as f64);
        let tickets = h.tickets as f64 + planned * prm.ticket_disc;

        let mut owned_others = 0u32;
        let mut produced_others = 0u32;
        for i in 0..h.np {
            if h.progs[i] != prog.id {
                owned_others += 1;
                if h.produced & (1 << i) != 0 {
                    produced_others += 1;
                }
            }
        }
        let prev = (produced_others + tm.prods_before_final).min(owned_others);
        let stars = self.stars_for(who, h, prog, other);
        let final_arena = h.arena as u32 + sq_need;
        let nobles = prm.noble_base
            + prm.noble_sq * (final_arena.saturating_sub(2)) as f64
            + if h.loge { prm.noble_loge } else { 0.0 };
        let score = base
            + (prev * s.previous_program_bonus) as f64
            + tickets * s.season_ticket_bonus as f64
            + (stars * s.star_award_bonus) as f64
            + (h.podiums * s.podium_bonus) as f64
            + nobles
            - sq_need as f64 * prm.step_penalty;
        Some(score - tm.lambda * (exp_cost + planned * tcost))
    }

    /// Total worth of a holding in points.
    pub fn value(&self, who: usize, h: &Hyp, tm: &Timing, other: Option<(usize, &[u32; NA])>) -> f64 {
        let mut best = 0.0f64;
        let mut second = 0.0f64;
        for i in 0..h.np {
            let Some(prog) = self.t.prog(h.progs[i]) else { continue };
            if let Some(v) = self.project(who, h, prog, tm, other) {
                if v > best {
                    second = best;
                    best = v;
                } else if v > second {
                    second = v;
                }
            }
        }
        let medal_pts = self.g.rules().scoring.medal_bonus as f64;
        let loge = if h.loge { self.p.loge_round * tm.prods_left as f64 } else { 0.0 };
        // Tickets and podiums also add spectators (coins) to every earlier event.
        let s = &self.g.rules().scoring;
        let per_event = (h.tickets * s.season_ticket_bonus + h.podiums * s.podium_bonus) as f64;
        let income = self.p.income_w * tm.lambda * per_event * tm.prods_before_final as f64;
        let tokens: u32 = h.a[..Asset::Joker.index()].iter().sum();
        best + self.p.second * second
            + income
            + self.p.token_w * tokens as f64
            + self.p.joker_w * h.a[Asset::Joker.index()] as f64
            + loge
            + h.medals as f64 * medal_pts
            + h.a[Asset::SpecialEvent.index()] as f64 * self.p.se_value
            + self.coin_pts(h.coins as f64, tm)
    }

    /// Points a stock of coins is worth (concave: the reserve counts more).
    pub fn coin_pts(&self, coins: f64, tm: &Timing) -> f64 {
        let r = tm.reserve;
        tm.lambda * (coins.min(r) * self.p.reserve_mult + (coins - r).max(0.0))
    }

    /// Points lost by paying `x` coins out of `coins`.
    pub fn pay_cost(&self, x: f64, coins: f64, tm: &Timing) -> f64 {
        self.coin_pts(coins, tm) - self.coin_pts((coins - x).max(0.0), tm)
    }

    /// The most coins worth giving up for `pts` points, holding `coins`.
    pub fn max_pay(&self, pts: f64, coins: f64, tm: &Timing) -> f64 {
        if pts <= 0.0 {
            return 0.0;
        }
        let l = tm.lambda.max(1e-6);
        let surplus = (coins - tm.reserve).max(0.0);
        let x = if pts <= l * surplus {
            pts / l
        } else {
            surplus + (pts - l * surplus) / (l * self.p.reserve_mult)
        };
        x.min(coins)
    }

    pub fn hyp(&self, who: usize) -> Hyp {
        Hyp::of(&self.g.players()[who])
    }


    /// Exact spectators for producing `prog` now with the given noble value
    /// in the arena and medals spent; `None` if it can't be produced.
    pub fn event_score(&self, who: usize, h: &Hyp, prog: &Prog, nobles: u32, medals: u32) -> Option<u32> {
        if h.arena < prog.squares {
            return None;
        }
        let base = prog.score_int(prog.missing(&h.a))?;
        let s = &self.g.rules().scoring;
        let mut prev = 0;
        for i in 0..h.np {
            if h.progs[i] != prog.id && h.produced & (1 << i) != 0 {
                prev += 1;
            }
        }
        let mut stars = 0;
        for a in 0..NA {
            if prog.req[a] > 0 && h.a[a] > 0 && self.holder[a] == Some(who) {
                stars += 1;
            }
        }
        Some(
            base + prev * s.previous_program_bonus
                + h.tickets * s.season_ticket_bonus
                + stars * s.star_award_bonus
                + nobles
                + h.podiums * s.podium_bonus
                + medals * s.medal_bonus,
        )
    }

    /// Best spectators producible now (nobles excluded), over owned programs.
    pub fn best_now(&self, who: usize, h: &Hyp) -> Option<(u32, u8)> {
        let mut best: Option<(u32, u8)> = None;
        for i in 0..h.np {
            let Some(prog) = self.t.prog(h.progs[i]) else { continue };
            if let Some(s) = self.event_score(who, h, prog, 0, 0)
                && best.is_none_or(|b| s > b.0)
            {
                best = Some((s, prog.id));
            }
        }
        best
    }

}
