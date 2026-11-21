//! Valuation shared by every phase: a projection of the score of a player's
//! final-round event from a (possibly hypothetical) holding. The same model
//! is applied to every seat, which is how chameleon reads its opponents.

use colosseum_engine::{Asset, Assets, EventDef, Game, Phase, PlayerId};

/// Most programs tracked per holding (more are never reached in practice).
pub const MAXP: usize = 16;

/// Tuning knobs (heuristics, not game rules).
#[derive(Clone, Debug)]
pub struct Params {
    /// Matching tokens expected from each remaining acquiring phase.
    pub gain_rate: f64,
    /// Points per coin in the first round (decays to the last).
    pub lambda: f64,
    pub lambda_min: f64,
    /// Coins expected from each production before the final round.
    pub income: f64,
    /// Expected noble spectators in the final event.
    pub nobles: f64,
    /// Discount on tickets merely planned.
    pub ticket_disc: f64,
    /// Fraction of future acquiring phases credited when valuing a batch.
    pub kappa: f64,
    /// Share of a batch's value I am willing to pay.
    pub pay_frac: f64,
    /// Points for producing a program not produced yet (non-final rounds).
    pub new_program: f64,
    /// Coins one useful token costs at auction.
    pub token_cost: f64,
    /// Points per outstanding arena expansion.
    pub step_penalty: f64,
    /// Points per remaining production for a loge.
    pub loge_per_round: f64,
    /// Share of a ticket's coin income per production that counts.
    pub ticket_income: f64,
    /// Points per medal held.
    pub medal_value: f64,
    /// Weight of denying the most threatening opponent.
    pub deny: f64,
    /// Weight of an opponent's gain when judging trades with them.
    pub rival_w: f64,
    /// Extra weight for the leader's gain.
    pub leader_w: f64,
    /// Smallest net gain to accept an offer.
    pub accept_min: f64,
    /// Weight of the second-best program.
    pub second_w: f64,
    /// Most trade proposals per trading turn.
    pub max_props: f64,
    /// Scale of final-round noble denial.
    pub final_deny: f64,
    /// Share of the best other lot's surplus deducted while bidding.
    pub alt_frac: f64,
    /// Whether to counter coin offers for my tokens (0 or 1).
    pub counter: f64,
    /// Share of a token's worth I'll pay when proposing.
    pub buy_frac: f64,
    /// Bonus per point of raising my best score with a non-final event.
    pub best_bump: f64,
    /// Points above their loss I expect a partner to need to accept.
    pub trade_margin: f64,
    /// Smallest score worth proposing.
    pub min_surplus: f64,
    /// Whether to propose token swaps / sales (0 or 1).
    pub swaps: f64,
    pub sells: f64,
    /// Initial per-opponent price scale and its decay after a quick yes.
    pub start_scale: f64,
    pub scale_down: f64,
    /// Rival willingness to pay, as a multiple of my model of their gain.
    pub rival_scale: f64,
    /// Weight of my cap (vs. surplus) when choosing a lot to open.
    pub open_cap_w: f64,
    /// Points per coin in the final round before production.
    pub lambda_last: f64,
    /// Share of a podium's bonus credited for taking the lead in best score.
    pub podium_w: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            gain_rate: 1.2,
            lambda: 0.4,
            lambda_min: 0.04,
            income: 8.0,
            nobles: 5.0,
            ticket_disc: 0.3,
            kappa: 0.0,
            pay_frac: 1.0,
            new_program: 3.0,
            token_cost: 6.0,
            step_penalty: 1.5,
            loge_per_round: 2.5,
            ticket_income: 0.8,
            medal_value: 3.0,
            deny: 0.6,
            rival_w: 0.6,
            leader_w: 1.2,
            accept_min: 3.0,
            second_w: 0.15,
            max_props: 12.0,
            final_deny: 1.0,
            alt_frac: 1.0,
            counter: 1.0,
            buy_frac: 1.0,
            best_bump: 0.2,
            trade_margin: 0.5,
            min_surplus: 0.3,
            swaps: 1.0,
            sells: 0.0,
            start_scale: 0.7,
            scale_down: 0.9,
            rival_scale: 1.0,
            open_cap_w: 0.1,
            lambda_last: 0.08,
            podium_w: 0.0,
        }
    }
}

impl Params {
    /// Sets a knob by name (for tuning runs). Returns false if unknown.
    pub fn set(&mut self, name: &str, v: f64) -> bool {
        match name {
            "gain_rate" => self.gain_rate = v,
            "lambda" => self.lambda = v,
            "lambda_min" => self.lambda_min = v,
            "income" => self.income = v,
            "nobles" => self.nobles = v,
            "ticket_disc" => self.ticket_disc = v,
            "kappa" => self.kappa = v,
            "pay_frac" => self.pay_frac = v,
            "new_program" => self.new_program = v,
            "token_cost" => self.token_cost = v,
            "step_penalty" => self.step_penalty = v,
            "loge_per_round" => self.loge_per_round = v,
            "ticket_income" => self.ticket_income = v,
            "medal_value" => self.medal_value = v,
            "deny" => self.deny = v,
            "rival_w" => self.rival_w = v,
            "leader_w" => self.leader_w = v,
            "accept_min" => self.accept_min = v,
            "second_w" => self.second_w = v,
            "max_props" => self.max_props = v,
            "final_deny" => self.final_deny = v,
            "alt_frac" => self.alt_frac = v,
            "counter" => self.counter = v,
            "buy_frac" => self.buy_frac = v,
            "best_bump" => self.best_bump = v,
            "trade_margin" => self.trade_margin = v,
            "min_surplus" => self.min_surplus = v,
            "swaps" => self.swaps = v,
            "sells" => self.sells = v,
            "start_scale" => self.start_scale = v,
            "scale_down" => self.scale_down = v,
            "rival_scale" => self.rival_scale = v,
            "open_cap_w" => self.open_cap_w = v,
            "lambda_last" => self.lambda_last = v,
            "podium_w" => self.podium_w = v,
            _ => return false,
        }
        true
    }

    /// Applies `name=value,...` overrides.
    pub fn with_overrides(mut self, spec: &str) -> Self {
        for kv in spec.split(',') {
            if let Some((k, v)) = kv.split_once('=')
                && let Ok(v) = v.trim().parse()
            {
                self.set(k.trim(), v);
            }
        }
        self
    }
}

/// A holding to evaluate.
#[derive(Clone, Debug)]
pub struct Hyp {
    pub assets: Assets,
    pub coins: u32,
    pub progs: [(u8, bool); MAXP],
    pub np: usize,
    pub arena_len: u8,
    pub tickets: u32,
    pub loge: bool,
    pub medals: u32,
    pub podiums: u32,
}

impl Hyp {
    pub fn of(g: &Game, p: PlayerId) -> Hyp {
        let pl = g.player(p);
        let mut h = Hyp {
            assets: pl.assets.clone(),
            coins: pl.coins,
            progs: [(0, false); MAXP],
            np: 0,
            arena_len: pl.arena.len,
            tickets: pl.season_tickets,
            loge: pl.loge,
            medals: pl.medals,
            podiums: pl.podiums,
        };
        for q in &pl.programs {
            if h.np < MAXP {
                h.progs[h.np] = (q.id, q.produced);
                h.np += 1;
            }
        }
        h
    }

    pub fn programs(&self) -> &[(u8, bool)] {
        &self.progs[..self.np]
    }

    pub fn add_program(&mut self, id: u8) {
        if self.np < MAXP {
            self.progs[self.np] = (id, false);
            self.np += 1;
        }
    }

    /// Tokens won at auction or donated: Emperor tokens become medals.
    pub fn add_tokens(&mut self, tokens: &[Asset], mut medal_supply: u32) {
        for &t in tokens {
            if t == Asset::Emperor {
                if medal_supply > 0 {
                    medal_supply -= 1;
                    self.medals += 1;
                }
            } else {
                self.assets.add(t, 1);
            }
        }
    }
}

/// How much of the game is left, for one player.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub slots: u32,
    pub acq_left: f64,
    pub prods_before_final: u32,
    pub lambda: f64,
}

/// Score with a fractional number of missing assets, interpolated.
pub fn base_score(def: &EventDef, missing: f64) -> f64 {
    let s = |k: usize| -> f64 {
        match def.score_with_missing(k) {
            Some(v) => v as f64,
            None => {
                let last = def.score_with_missing(def.max_missing()).unwrap_or(0) as f64;
                (last - 6.0 * (k - def.max_missing()) as f64).max(0.0)
            }
        }
    };
    let lo = missing.floor().max(0.0) as usize;
    let frac = missing - lo as f64;
    if frac < 1e-9 {
        return s(lo);
    }
    s(lo) * (1.0 - frac) + s(lo + 1) * frac
}

/// Requirement counts per program, cached once per decision.
pub struct Req {
    pub id: u8,
    /// Index into `config().events`.
    pub idx: usize,
    pub kinds: [(Asset, u32); 12],
    pub nk: usize,
}

pub struct Model<'a> {
    pub g: &'a Game,
    pub prm: &'a Params,
    /// Per asset: the highest count held, its holder, and the second highest.
    top: [(u32, usize, u32); Asset::COUNT],
    star_asset: [bool; Asset::COUNT],
    star_holder: [Option<usize>; Asset::COUNT],
    reqs: &'a [Req],
}

/// Builds the requirement table for a config.
pub fn build_reqs(g: &Game) -> Vec<Req> {
    g.config()
        .events
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            let counts = e.requirement_counts();
            let mut kinds = [(Asset::Gladiator, 0u32); 12];
            let mut nk = 0;
            for (a, n) in counts.iter() {
                if nk < 12 {
                    kinds[nk] = (a, n);
                    nk += 1;
                }
            }
            Req { id: e.id, idx, kinds, nk }
        })
        .collect()
}

impl<'a> Model<'a> {
    pub fn new(g: &'a Game, prm: &'a Params, reqs: &'a [Req]) -> Self {
        let mut top = [(0u32, usize::MAX, 0u32); Asset::COUNT];
        for (i, p) in g.players().iter().enumerate() {
            for a in Asset::ALL {
                let c = p.assets.count(a);
                let t = &mut top[a.index()];
                if c > t.0 {
                    t.2 = t.0;
                    *t = (c, i, t.2);
                } else if c > t.2 {
                    t.2 = c;
                }
            }
        }
        let mut star_asset = [false; Asset::COUNT];
        for a in &g.rules().stars.assets {
            star_asset[a.index()] = true;
        }
        let mut star_holder = [None; Asset::COUNT];
        for (a, h) in g.stars() {
            star_holder[a.index()] = h.map(|p| p.0);
        }
        Model {
            g,
            prm,
            top,
            star_asset,
            star_holder,
            reqs,
        }
    }

    pub fn req(&self, id: u8) -> Option<&'a Req> {
        // Events are listed by id 1..; fall back to a scan.
        let i = id as usize;
        if i >= 1 && i <= self.reqs.len() && self.reqs[i - 1].id == id {
            return Some(&self.reqs[i - 1]);
        }
        self.reqs.iter().find(|r| r.id == id)
    }

    pub fn def(&self, id: u8) -> Option<(&'a EventDef, &'a Req)> {
        let r = self.req(id)?;
        let g: &'a Game = self.g;
        g.config().events.get(r.idx).map(|d| (d, r))
    }

    /// Highest count of `asset` held by anyone but `who`.
    pub fn other_max(&self, who: usize, asset: Asset) -> u32 {
        let t = self.top[asset.index()];
        if t.1 == who { t.2 } else { t.0 }
    }

    pub fn holds_star(&self, who: usize, asset: Asset, count: u32) -> bool {
        if !self.star_asset[asset.index()] || count < self.g.rules().stars.threshold {
            return false;
        }
        let o = self.other_max(who, asset);
        count > o || (count == o && self.star_holder[asset.index()] == Some(who))
    }

    /// Points per coin now.
    pub fn lambda(&self) -> f64 {
        let g = self.g;
        let total = g.rules().num_rounds.max(1) as f64;
        if g.round() >= g.rules().num_rounds {
            if matches!(g.phase(), Phase::Producing(_) | Phase::Closing(_) | Phase::GameOver) {
                return self.prm.lambda_min;
            }
            return self.prm.lambda_last;
        }
        let left = (total - g.round() as f64 + 1.0).max(0.0);
        (self.prm.lambda * left / total).max(self.prm.lambda_min)
    }

    /// Timing for player `who` at the current point of the game.
    pub fn timing(&self, who: PlayerId) -> Timing {
        let g = self.g;
        let r = g.round();
        let total = g.rules().num_rounds;
        let later = total.saturating_sub(r);
        let (slots, acq_left) = match g.phase() {
            Phase::Investing { turn, remaining } => {
                let n = g.num_players();
                let my_turn = (who.0 + n - g.start_player().0) % n;
                let now = if my_turn > *turn {
                    1
                } else if my_turn == *turn {
                    *remaining
                } else {
                    0
                };
                (now + later, (later + 1) as f64)
            }
            Phase::Acquiring(_) => (later, later as f64),
            _ => (later, later as f64),
        };
        let mut prods = later;
        if let Phase::Producing(pr) = g.phase() {
            let n = g.num_players();
            let my_turn = (who.0 + n - g.start_player().0) % n;
            if my_turn < pr.turn {
                prods = prods.saturating_sub(1);
            }
        } else if matches!(g.phase(), Phase::Closing(_)) {
            prods = prods.saturating_sub(1);
        }
        Timing {
            slots,
            acq_left,
            prods_before_final: prods,
            lambda: self.lambda(),
        }
    }

    /// Projected final score of `def` and the expansion cost to reach it.
    pub fn target_score(&self, who: usize, def: &EventDef, h: &Hyp, t: &Timing) -> Option<(f64, f64)> {
        let rules = self.g.rules();
        let req = self.req(def.id)?;
        let squares = *rules.arena.squares_required_by_event_size.get(def.size as usize)? as u32;
        let sq_def = squares.saturating_sub(h.arena_len as u32);
        if sq_def > t.slots {
            return None;
        }
        let exp_cost = (sq_def * rules.arena.expansion_cost) as f64;
        let budget = h.coins as f64 + self.prm.income * t.prods_before_final as f64;
        if exp_cost > budget {
            return None;
        }
        let s = &rules.scoring;
        let free_slots = t.slots - sq_def;
        let ticket_cost = rules.investing.season_ticket_cost.max(1) as f64;
        let afford = ((budget - exp_cost) / ticket_cost).max(0.0);
        let planned = (free_slots as f64)
            .min(afford)
            .min(self.g.supply().season_tickets as f64);
        let tickets = h.tickets as f64 + planned * self.prm.ticket_disc;
        let spare = (budget - exp_cost - planned * ticket_cost).max(0.0);
        let gain = (self.prm.gain_rate * t.acq_left).min(spare / self.prm.token_cost);

        let mut missing = 0u32;
        let mut stars = 0u32;
        for &(a, n) in &req.kinds[..req.nk] {
            let have = h.assets.count(a);
            missing += n.saturating_sub(have);
            if self.holds_star(who, a, have) {
                stars += 1;
            }
        }
        let missing = missing.saturating_sub(h.assets.count(Asset::Joker)) as f64;
        let base = base_score(def, (missing - gain).max(0.0));

        let mut owned_others = 0u32;
        let mut produced_others = 0u32;
        for &(id, p) in h.programs() {
            if id != def.id {
                owned_others += 1;
                produced_others += p as u32;
            }
        }
        let prev = (produced_others + t.prods_before_final).min(owned_others);

        let score = base
            + (prev * s.previous_program_bonus) as f64
            + tickets * s.season_ticket_bonus as f64
            + (stars * s.star_award_bonus) as f64
            + self.prm.nobles
            + h.medals as f64 * self.prm.medal_value
            + (h.podiums * s.podium_bonus) as f64
            + h.tickets as f64 * (s.season_ticket_bonus * t.prods_before_final) as f64 * t.lambda * self.prm.ticket_income
            - sq_def as f64 * self.prm.step_penalty;
        Some((score, exp_cost))
    }

    /// Best and second-best `(score - cost * lambda)` over owned programs,
    /// plus the best raw score.
    fn programs_eval(&self, who: usize, h: &Hyp, t: &Timing) -> (f64, f64, f64) {
        let mut best = 0.0f64;
        let mut second = 0.0f64;
        let mut raw = 0.0f64;
        for &(id, _) in h.programs() {
            if let Some((def, _)) = self.def(id)
                && let Some((score, cost)) = self.target_score(who, def, h, t)
            {
                let v = score - cost * t.lambda;
                raw = raw.max(score);
                if v > best {
                    second = best;
                    best = v;
                } else if v > second {
                    second = v;
                }
            }
        }
        (best, second, raw)
    }

    /// Overall worth of a holding, in points.
    pub fn value(&self, who: usize, h: &Hyp, t: &Timing) -> f64 {
        let (best, second, _) = self.programs_eval(who, h, t);
        let loge = if h.loge {
            self.prm.loge_per_round * (t.prods_before_final + 1) as f64
        } else {
            0.0
        };
        let se = h.assets.count(Asset::SpecialEvent) as f64 * self.prm.medal_value * 0.8;
        best + self.prm.second_w * second + loge + se + h.coins as f64 * t.lambda
    }

    /// Projected final score (no coin terms), at least the best so far.
    pub fn projection(&self, who: PlayerId) -> f64 {
        let h = Hyp::of(self.g, who);
        let t = self.timing(who);
        let (_, _, raw) = self.programs_eval(who.0, &h, &t);
        raw.max(self.g.player(who).best_score as f64)
    }
}

/// Every player's projected final score.
pub fn projections(m: &Model) -> Vec<f64> {
    (0..m.g.num_players()).map(|i| m.projection(PlayerId(i))).collect()
}

/// How much `q`'s gains should count against me, from projected scores.
pub fn threat_weights(m: &Model, me: PlayerId) -> Vec<f64> {
    let proj = projections(m);
    let n = proj.len();
    let best_other = (0..n)
        .filter(|&i| i != me.0)
        .map(|i| proj[i])
        .fold(f64::NEG_INFINITY, f64::max);
    (0..n)
        .map(|i| {
            if i == me.0 {
                return 0.0;
            }
            let mut w = m.prm.rival_w;
            if proj[i] >= best_other - 1e-9 {
                w += m.prm.leader_w;
            }
            // Players far behind me matter little; those ahead matter more.
            let gap = proj[i] - proj[me.0];
            w * (1.0 + (gap / 15.0).clamp(-0.8, 1.0))
        })
        .collect()
}
