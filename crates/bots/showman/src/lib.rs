//! Showman: a producing-phase and bonus specialist.
//!
//! Strategy: plan noble moves (dice, combined, medal) with a small search
//! that values the event score, medals earned, and the noble bonuses it
//! hands to rivals; spend medals on spectators in the final round; keep
//! star awards and useful tokens when discarding; other phases lean on
//! the greedy heuristics. Investing and acquiring use an estimate of the
//! best reachable event.

mod acquire;
mod invest;
mod produce;

use colosseum_engine::agent::{Agent, GreedyAgent};
use colosseum_engine::{Action, Game, Phase, PlayerId};

pub struct ShowmanAgent {
    inner: GreedyAgent,
    tune: invest::Tune,
    acq: acquire::AcqTune,
}

impl ShowmanAgent {
    pub fn new(seed: u64) -> Self {
        Self { inner: GreedyAgent::new(seed), tune: invest::Tune::default(), acq: acquire::AcqTune::default() }
    }
}

/// How good a player's holdings are for producing: best event now, plus a
/// little credit for every other program and held stars.
fn potential(game: &Game, p: PlayerId) -> f64 {
    let mut scores: Vec<u32> = game
        .player(p)
        .programs
        .iter()
        .filter_map(|pr| game.event_preview(p, pr.id, 0))
        .map(|pv| pv.breakdown.total())
        .collect();
    scores.sort_unstable_by(|a, b| b.cmp(a));
    let best = scores.first().copied().unwrap_or(0) as f64;
    let rest: f64 = scores.iter().skip(1).map(|&s| s as f64).sum();
    best + 0.1 * rest + game.stars_held(p) as f64 * 0.5
}

/// How much the current holdings help reach the player's programs, counting
/// the tokens still missing for each (a looser measure than `potential`
/// that also rewards partial progress).
fn closeness(game: &Game, p: PlayerId) -> f64 {
    let me = game.player(p);
    let mut best = 0.0f64;
    for pr in &me.programs {
        let Some(def) = game.config().event(pr.id) else { continue };
        let need = def.requirement_counts();
        let mut have = 0u32;
        let mut total = 0u32;
        for (a, n) in need.iter() {
            total += n;
            have += me.assets.count(a).min(n);
        }
        let frac = if total == 0 { 0.0 } else { have as f64 / total as f64 };
        best = best.max(frac * def.base_score as f64);
    }
    best
}

fn value_after(game: &Game, me: PlayerId, action: &Action) -> f64 {
    let mut next = game.clone();
    if next.apply(action.clone()).is_err() {
        return f64::NEG_INFINITY;
    }
    potential(&next, me) + 0.3 * closeness(&next, me)
}

impl ShowmanAgent {
    fn close(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Closing(c) = game.phase() else {
            return legal[0].clone();
        };
        let donor = c.donation.map(|(_, d)| d);
        let mut best: Option<(f64, Action)> = None;
        for a in legal {
            if !matches!(a, Action::DiscardAsset { .. } | Action::RequestAsset { .. }) {
                continue;
            }
            let mut v = value_after(game, me, a);
            if matches!(a, Action::RequestAsset { .. })
                && let Some(d) = donor
            {
                let mut next = game.clone();
                if next.apply(a.clone()).is_ok() {
                    v -= 0.4 * potential(&next, d);
                }
            }
            if best.as_ref().is_none_or(|b| v > b.0) {
                best = Some((v, a.clone()));
            }
        }
        best.map_or_else(|| legal[0].clone(), |b| b.1)
    }
}

impl Agent for ShowmanAgent {
    fn name(&self) -> String {
        "showman".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
        match game.phase() {
            Phase::Producing(_) => {
                let last_round = game.round() >= game.rules().num_rounds;
                // Cash a SpecialEvent token into a medal before the final event.
                if last_round && legal.contains(&Action::ExchangeSpecialEventForMedal) {
                    return Action::ExchangeSpecialEventForMedal;
                }
                if let Some(a) = produce::plan(game, legal) {
                    return a;
                }
                self.inner.choose_action(game, legal)
            }
            Phase::Closing(_) => self.close(game, me, legal),
            Phase::Acquiring(_) => acquire::choose(game, me, legal, &self.acq),
            Phase::Investing { remaining, .. } => {
                if *remaining == 0 {
                    if legal.contains(&Action::PlaySpecialEvent) && invest::special_pays(game, me, &self.tune) {
                        return Action::PlaySpecialEvent;
                    }
                    return Action::EndInvesting;
                }
                invest::choose(game, me, legal, &self.tune)
            }
            _ => {
                let a = self.inner.choose_action(game, legal);
                if legal.contains(&a) { a } else { legal[0].clone() }
            }
        }
    }
}
