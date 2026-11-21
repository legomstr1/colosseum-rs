//! Economic valuation: how many points of expected final event score a
//! player's holdings are worth, and so what a lot, a program or a trade is
//! worth in the same currency.

use colosseum_engine::config::EventDef;
use colosseum_engine::{Asset, Assets, Game, PlayerId, PlayerState, Phase};

/// Tunable heuristics (not game data: rules come from the engine config).
#[derive(Clone, Debug)]
pub struct Params {
    /// Useful tokens expected from each remaining acquiring phase.
    pub gain_per_round: f64,
    /// Weight of the second-best program in the potential.
    pub second_weight: f64,
    /// Points lost per missing arena square, as a fraction of its cost.
    pub arena_pen: f64,
    /// Points per coin in the first round...
    pub lambda_first: f64,
    /// ...and in the last round.
    pub lambda_last: f64,
    /// Weight of denying an opponent a lot.
    pub deny: f64,
    /// Value of an Emperor loge, in points, over the whole game.
    pub loge_pts: f64,
    /// Share of the best alternative lot's surplus deducted from a lot's
    /// value (winning a lot ends my acquiring turn).
    pub alt_frac: f64,
    /// Value of a coin reserve to keep when bidding.
    pub reserve: u32,
    /// Coins beyond this are worth only `tail` of a normal coin.
    pub wealth: f64,
    /// Coin prices offered for a token, in rising order.
    pub prices: Vec<u32>,
    /// Most offers made per trading turn.
    pub max_offers: u32,
    /// Smallest gain that justifies an investment.
    pub invest_min: f64,
    /// Smallest investment gain that justifies a SpecialEvent token...
    pub extra_min: f64,
    /// ...or two medals.
    pub medal_extra_min: f64,
    pub tail: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            gain_per_round: 0.0,
            second_weight: 0.1,
            arena_pen: 1.0,
            lambda_first: 0.35,
            lambda_last: 0.15,
            deny: 0.25,
            loge_pts: 6.0,
            alt_frac: 1.0,
            reserve: 0,
            wealth: 30.0,
            prices: vec![2, 3, 5, 7, 10, 14],
            max_offers: 8,
            invest_min: 0.5,
            extra_min: 2.0,
            medal_extra_min: 9.0,
            tail: 0.25,
        }
    }
}

/// A player's holdings, editable for what-if questions.
#[derive(Clone, Debug)]
pub struct Hyp {
    pub assets: Assets,
    /// `(program id, produced)`.
    pub programs: Vec<(u8, bool)>,
    pub arena_len: u8,
    pub tickets: u32,
    pub podiums: u32,
    pub medals: u32,
    pub coins: u32,
    pub loge: bool,
}

impl Hyp {
    pub fn of(p: &PlayerState) -> Hyp {
        Hyp {
            assets: p.assets.clone(),
            programs: p.programs.iter().map(|q| (q.id, q.produced)).collect(),
            arena_len: p.arena.len,
            tickets: p.season_tickets,
            podiums: p.podiums,
            medals: p.medals,
            coins: p.coins,
            loge: p.loge,
        }
    }

    pub fn add_tokens(&mut self, game: &Game, tokens: &[Asset]) {
        let mut medals_left = game.supply().medals;
        for &t in tokens {
            if t == Asset::Emperor {
                if medals_left > 0 {
                    medals_left -= 1;
                    self.medals += 1;
                }
            } else {
                self.assets.add(t, 1);
            }
        }
    }
}

pub struct Eval<'a> {
    pub game: &'a Game,
    pub prm: &'a Params,
    /// Acquiring phases still to come (including a current one).
    pub acq_left: f64,
    /// Production rounds still to come (including a current one).
    pub prod_left: u32,
    pub lambda: f64,
}

impl<'a> Eval<'a> {
    pub fn new(game: &'a Game, prm: &'a Params) -> Self {
        let total = game.rules().num_rounds;
        let round = game.round();
        let (acq_left, prod_left) = match game.phase() {
            Phase::Investing { .. } | Phase::Acquiring(_) => (total - round + 1, total - round + 1),
            Phase::Trading { .. } | Phase::Producing(_) => (total - round, total - round + 1),
            _ => (total.saturating_sub(round), total.saturating_sub(round)),
        };
        let span = total.saturating_sub(1).max(1) as f64;
        let t = ((round.saturating_sub(1)) as f64 / span).min(1.0);
        let lambda = prm.lambda_first + (prm.lambda_last - prm.lambda_first) * t;
        Eval {
            game,
            prm,
            acq_left: acq_left as f64,
            prod_left,
            lambda,
        }
    }

    pub fn medal_pts(&self) -> f64 {
        self.game.rules().scoring.medal_bonus as f64
    }

    fn other_max(&self, who: PlayerId, asset: Asset) -> u32 {
        self.game
            .players()
            .iter()
            .filter(|p| p.id != who)
            .map(|p| p.assets.count(asset))
            .max()
            .unwrap_or(0)
    }

    /// Expected score of producing `def` at the end of the game.
    pub fn est(&self, who: PlayerId, h: &Hyp, def: &EventDef) -> f64 {
        let rules = self.game.rules();
        let s = &rules.scoring;
        let need = def.requirement_counts();
        let mut missing = 0u32;
        for (a, n) in need.iter() {
            missing += n.saturating_sub(h.assets.count(a));
        }
        missing = missing.saturating_sub(h.assets.count(Asset::Joker));
        let em = (missing as f64 - self.prm.gain_per_round * self.acq_left).max(0.0);
        let k = em.floor() as usize;
        let frac = em - em.floor();
        let at = |m: usize| -> f64 {
            match def.score_with_missing(m) {
                Some(v) => v as f64,
                None => {
                    let last = def.score_with_missing(def.max_missing()).unwrap_or(0) as f64;
                    (last - 3.0 * (m - def.max_missing()) as f64).max(0.0)
                }
            }
        };
        let base = at(k) * (1.0 - frac) + at(k + 1) * frac;

        // Stars
        let thr = rules.stars.threshold;
        let mut stars = 0u32;
        for &a in &rules.stars.assets {
            if need.count(a) == 0 {
                continue;
            }
            let mine = h.assets.count(a);
            let om = self.other_max(who, a);
            let holder = self.game.stars().get(&a).copied().flatten();
            if mine >= thr && (mine > om || (mine == om && holder == Some(who))) {
                stars += 1;
            }
        }

        let others = h.programs.iter().filter(|(id, _)| *id != def.id).count() as u32;
        let produced_others = h.programs.iter().filter(|(id, pr)| *id != def.id && *pr).count() as u32;
        let unproduced = others - produced_others;
        let prev = produced_others + unproduced.min(self.prod_left.saturating_sub(1));

        let required = rules
            .arena
            .squares_required_by_event_size
            .get(def.size as usize)
            .copied()
            .unwrap_or(0);
        let shortfall = (required as f64 - h.arena_len as f64).max(0.0);

        base + (prev * s.previous_program_bonus) as f64
            + (h.tickets * s.season_ticket_bonus) as f64
            + (stars * s.star_award_bonus) as f64
            + (h.podiums * s.podium_bonus) as f64
            - shortfall * rules.arena.expansion_cost as f64 * self.prm.arena_pen
    }

    /// Expected best event score, in points.
    pub fn potential(&self, who: PlayerId, h: &Hyp) -> f64 {
        let mut best = 0.0f64;
        let mut second = 0.0f64;
        for &(id, _) in &h.programs {
            if let Some(def) = self.game.config().event(id) {
                let v = self.est(who, h, def);
                if v > best {
                    second = best;
                    best = v;
                } else if v > second {
                    second = v;
                }
            }
        }
        let loge = if h.loge {
            self.prm.loge_pts * self.prod_left as f64 / self.game.rules().num_rounds as f64
        } else {
            0.0
        };
        best + self.prm.second_weight * second + loge + h.medals as f64 * self.medal_pts() * 0.8
    }

    /// Total worth in points: potential plus coins.
    pub fn worth(&self, who: PlayerId, h: &Hyp) -> f64 {
        self.potential(who, h) + self.coin_pts(h.coins as f64)
    }

    /// Points a stock of coins is worth (concave: surplus coins are cheap).
    pub fn coin_pts(&self, coins: f64) -> f64 {
        let w = self.prm.wealth;
        self.lambda * coins.min(w) + self.lambda * self.prm.tail * (coins - w).max(0.0)
    }

    /// The most coins I'd give up for `pts` points, holding `coins` now.
    pub fn coins_for_pts(&self, pts: f64, coins: f64) -> f64 {
        if pts <= 0.0 {
            return 0.0;
        }
        let w = self.prm.wealth;
        let lam = self.lambda.max(1e-6);
        let surplus = (coins - w).max(0.0);
        let tail_pts = surplus * lam * self.prm.tail;
        if pts <= tail_pts {
            pts / (lam * self.prm.tail)
        } else {
            surplus + (pts - tail_pts) / lam
        }
    }

    /// Points gained by `who` from winning `lot` for free.
    pub fn lot_delta(&self, who: PlayerId, lot: &[Asset]) -> f64 {
        let h = Hyp::of(self.game.player(who));
        let before = self.potential(who, &h);
        let mut h2 = h;
        h2.add_tokens(self.game, lot);
        let mut extra = 0.0;
        for &t in lot {
            if t == Asset::SpecialEvent {
                extra += 0.6 * self.medal_pts();
            }
        }
        self.potential(who, &h2) - before + extra
    }
}
