//! The planner's valuation model: projects how well a player could do in the
//! final round from a (possibly hypothetical) holding of tokens, programs,
//! arena squares and tickets. Everything numeric about the game comes from
//! `game.rules()` / `game.config()`; only the planner's own tuning lives in
//! [`Params`].

use colosseum_engine::{Asset, Assets, EventDef, Game, PlayerId};

/// The planner's own tuning knobs (not game rules).
#[derive(Clone, Debug)]
pub struct Params {
    /// Matching tokens expected from each remaining acquiring phase.
    pub gain_rate: f64,
    /// Points per coin early in the game.
    pub lambda: f64,
    /// Coins expected from each production before the final round.
    pub income: f64,
    /// Expected noble spectators in the final event.
    pub nobles: f64,
    /// Discount on tickets we merely plan to buy later.
    pub ticket_disc: f64,
    /// Unused legacy knob (see `loge_per_round`).
    pub loge_value: f64,
    /// Fraction of future acquiring phases credited when valuing a batch.
    pub kappa: f64,
    /// Share of a batch's value we are willing to pay.
    pub pay_frac: f64,
    /// Points for producing a program we have not produced yet.
    pub new_program: f64,
    /// Coins one useful token costs at auction.
    pub token_cost: f64,
    /// Points per outstanding arena expansion (risk of never doing it).
    pub step_penalty: f64,
    /// Points per remaining production for owning a loge.
    pub loge_per_round: f64,
    /// Fraction of a ticket's coin income per production that counts.
    pub ticket_income: f64,
    /// Points per medal held at the end.
    pub medal_value: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            gain_rate: 1.2,
            lambda: 0.4,
            income: 12.0,
            nobles: 5.0,
            ticket_disc: 0.6,
            loge_value: 4.0,
            kappa: 0.5,
            pay_frac: 0.8,
            new_program: 3.0,
            medal_value: 3.0,
            token_cost: 6.0,
            step_penalty: 1.5,
            loge_per_round: 2.5,
            ticket_income: 0.8,
        }
    }
}

/// A holding to evaluate: the real one, or the real one after some change.
#[derive(Clone, Debug)]
pub struct Hyp {
    pub assets: Assets,
    pub coins: u32,
    /// `(program id, produced yet)`.
    pub owned: Vec<(u8, bool)>,
    pub arena_len: u8,
    pub tickets: u32,
    pub loge: bool,
    pub medals: u32,
    pub podiums: u32,
}

impl Hyp {
    pub fn of(game: &Game, me: PlayerId) -> Hyp {
        let p = game.player(me);
        Hyp {
            assets: p.assets.clone(),
            coins: p.coins,
            owned: p.programs.iter().map(|q| (q.id, q.produced)).collect(),
            arena_len: p.arena.len,
            tickets: p.season_tickets,
            loge: p.loge,
            medals: p.medals,
            podiums: p.podiums,
        }
    }
}

/// How much of the game is left.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Investment actions still available (this turn's and later rounds').
    pub slots: u32,
    /// Acquiring phases still to come that can help the final event.
    pub acq_left: f64,
    /// Productions before the final round's.
    pub prods_before_final: u32,
    /// Points per coin now.
    pub lambda: f64,
}

pub struct Model<'a> {
    pub game: &'a Game,
    pub me: PlayerId,
    pub params: &'a Params,
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
    s(lo) * (1.0 - frac) + s(lo + 1) * frac
}

/// Number of required assets missing, after Jokers fill in.
pub fn missing_count(def: &EventDef, assets: &Assets) -> u32 {
    let mut missing = 0;
    for (a, n) in def.requirement_counts().iter() {
        missing += n.saturating_sub(assets.count(a));
    }
    missing.saturating_sub(assets.count(Asset::Joker))
}

impl Model<'_> {
    pub fn timing_now(&self, in_investing: bool, remaining_now: u32) -> Timing {
        let r = self.game.round();
        let total = self.game.rules().num_rounds;
        let later = total.saturating_sub(r);
        let (slots, acq_left) = if in_investing {
            (remaining_now + later, (later + 1) as f64)
        } else {
            (later, later as f64)
        };
        Timing {
            slots,
            acq_left,
            prods_before_final: later,
            lambda: self.lambda(),
        }
    }

    /// Points per coin: coins fund bids and purchases, so they are worth
    /// less as the game runs out.
    pub fn lambda(&self) -> f64 {
        let total = self.game.rules().num_rounds.max(1) as f64;
        let left = (total - self.game.round() as f64 + 1.0).max(0.0);
        (self.params.lambda * left / total).max(0.04)
    }

    fn holds_star(&self, asset: Asset, my_count: u32) -> bool {
        let rules = self.game.rules();
        if !rules.stars.assets.contains(&asset) || my_count < rules.stars.threshold {
            return false;
        }
        let others = self
            .game
            .players()
            .iter()
            .filter(|p| p.id != self.me)
            .map(|p| p.assets.count(asset))
            .max()
            .unwrap_or(0);
        if my_count > others {
            return true;
        }
        // Ties keep the award with its current holder.
        my_count == others && self.game.stars().get(&asset).copied().flatten() == Some(self.me)
    }

    /// Projected score of the final event if `def` is the target.
    /// `None` when it cannot be produced in time.
    pub fn target_score(&self, def: &EventDef, h: &Hyp, t: &Timing) -> Option<(f64, f64)> {
        let rules = self.game.rules();
        let squares = *rules
            .arena
            .squares_required_by_event_size
            .get(def.size as usize)? as u32;
        let sq_def = squares.saturating_sub(h.arena_len as u32);
        if sq_def > t.slots {
            return None;
        }
        let exp_cost = (sq_def * rules.arena.expansion_cost) as f64;
        let budget = h.coins as f64 + self.params.income * t.prods_before_final as f64;
        if exp_cost > budget {
            return None;
        }

        let s = &rules.scoring;
        let free_slots = t.slots - sq_def;
        let ticket_cost = rules.investing.season_ticket_cost.max(1) as f64;
        let afford = ((budget - exp_cost) / ticket_cost).max(0.0);
        let planned = (free_slots as f64).min(afford).min(self.game.supply().season_tickets as f64);
        let extra_tickets = planned * self.params.ticket_disc;
        let tickets = h.tickets as f64 + extra_tickets;
        // What is left after tickets pays for tokens at auction.
        let spare = (budget - exp_cost - planned * ticket_cost).max(0.0);
        let gain = (self.params.gain_rate * t.acq_left).min(spare / self.params.token_cost);

        let missing = missing_count(def, &h.assets) as f64;
        let mf = (missing - gain).max(0.0);
        let base = base_score(def, mf);

        let owned_others = h.owned.iter().filter(|(id, _)| *id != def.id).count() as u32;
        let produced_others = h.owned.iter().filter(|(id, p)| *id != def.id && *p).count() as u32;
        let prev = (produced_others + t.prods_before_final).min(owned_others);

        let mut stars = 0u32;
        for (a, _) in def.requirement_counts().iter() {
            if self.holds_star(a, h.assets.count(a)) {
                stars += 1;
            }
        }

        let score = base
            + (prev * s.previous_program_bonus) as f64
            + tickets * s.season_ticket_bonus as f64
            + (stars * s.star_award_bonus) as f64
            + self.params.nobles
            + h.medals as f64 * self.params.medal_value
            + (h.podiums * s.podium_bonus) as f64
            + h.tickets as f64 * (s.season_ticket_bonus * t.prods_before_final) as f64 * t.lambda * self.params.ticket_income
            - sq_def as f64 * self.params.step_penalty;
        Some((score, exp_cost))
    }

    /// Overall worth of a holding, in points.
    pub fn value(&self, h: &Hyp, t: &Timing) -> f64 {
        let mut best = 0.0f64;
        for (id, _) in &h.owned {
            if let Some(def) = self.game.config().event(*id)
                && let Some((score, cost)) = self.target_score(def, h, t)
            {
                best = best.max(score - cost * t.lambda);
            }
        }
        let loge = if h.loge { self.params.loge_per_round * (t.prods_before_final + 1) as f64 } else { 0.0 };
        best + loge + h.coins as f64 * t.lambda
    }
}
