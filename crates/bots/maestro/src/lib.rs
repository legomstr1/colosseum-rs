//! Maestro: one valuation model, used everywhere.
//!
//! Every holding (tokens, programs, arena, tickets, loge, medals, podiums,
//! coins) is priced in points of projected final-event score
//! ([`model::Ctx::value`]). The same model, applied to opponents, prices
//! denial in auctions and judges trades from both sides:
//!
//! - investing: the investment that raises the projection most;
//! - acquiring: bid up to the lot's worth to me plus a share of its worth
//!   to the strongest rival still bidding, less the surplus of the best
//!   alternative lot;
//! - trading: buy needed tokens for coins on a rising price ladder and
//!   offer swaps; accept offers only when my gain clearly beats the
//!   partner's;
//! - producing: exhaustive dice search over noble moves (medals, nobles in
//!   the arena, denial in the final round), then the event that is best for
//!   coins, podiums and the previous-programs bonus;
//! - closing: discard and request by the same valuation.

mod model;
mod phases;
mod tables;
mod trade;

use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Game, Phase};
use model::Ctx;
use tables::Tables;

/// Maestro's own tuning knobs (heuristics, not rules).
#[derive(Clone, Debug)]
pub struct Params {
    /// Points per coin early in the game and in the second-to-last round.
    pub lam_early: f64,
    pub lam_late: f64,
    /// Points per coin in the final round's investing, acquiring and
    /// trading phases, and after them.
    pub lam_last_inv: f64,
    pub lam_last_acq: f64,
    pub lam_last_trade: f64,
    pub lam_end: f64,
    /// Share of the current acquiring phase still to come for a player who
    /// has not won a lot yet.
    pub acq_now: f64,
    /// Coins expected from each production before the final round.
    pub income: f64,
    /// Useful tokens expected per acquiring phase, and their coin price.
    pub gain_rate: f64,
    pub token_cost: f64,
    /// Credit for unproduced other programs (previous-programs bonus).
    pub prev_disc: f64,
    pub star_w: f64,
    /// Points per missing arena square on top of its coin cost.
    pub slot_pen: f64,
    pub second_w: f64,
    /// Points per remaining production for owning a loge.
    pub loge_round: f64,
    /// Expected noble spectators per arena square.
    pub nob_sq: f64,
    pub medal_val: f64,
    pub special_val: f64,
    pub joker_extra: f64,
    /// Weight of a rival's gain when bidding (denial).
    pub deny: f64,
    /// Share of the best alternative lot's surplus deducted from a bid cap.
    pub alt_frac: f64,
    /// Weight of a partner's gain when judging trades.
    pub rho: f64,
    pub acc_min: f64,
    pub trade_min: f64,
    pub buy_min: f64,
    pub max_offers: f64,
    pub swaps: f64,
    /// Producing: bonus for a first production of a non-target program,
    /// and the worth of taking the podium lead.
    pub new_prog: f64,
    pub podium_val: f64,
    pub deny_noble: f64,
    pub invest_min: f64,
    pub se_lock: f64,
    /// Whether to learn partners' prices and offer less to refusers.
    pub learn: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            lam_early: 0.35,
            lam_late: 0.12,
            lam_last_inv: 0.2,
            lam_last_acq: 0.12,
            lam_last_trade: 0.06,
            lam_end: 0.005,
            acq_now: 0.6,
            income: 10.0,
            gain_rate: 0.2,
            token_cost: 6.0,
            prev_disc: 0.85,
            star_w: 0.8,
            slot_pen: 1.5,
            second_w: 0.3,
            loge_round: 2.0,
            nob_sq: 3.0,
            medal_val: 3.0,
            special_val: 2.5,
            joker_extra: 0.3,
            deny: 0.5,
            alt_frac: 1.5,
            rho: 0.5,
            acc_min: 1.5,
            trade_min: 0.0,
            buy_min: 0.8,
            max_offers: 25.0,
            swaps: 1.0,
            new_prog: 4.0,
            podium_val: 2.0,
            deny_noble: 0.2,
            invest_min: 0.2,
            se_lock: 2.0,
            learn: 2.0,
        }
    }
}

macro_rules! params_set {
    ($self:ident, $k:ident, $v:ident, $($f:ident),*) => {
        match $k {
            $(stringify!($f) => { $self.$f = $v; true })*
            _ => false,
        }
    };
}

impl Params {
    /// Sets a knob by name; false if there is no such knob.
    pub fn set(&mut self, k: &str, v: f64) -> bool {
        params_set!(
            self, k, v, lam_early, lam_late, lam_last_inv, lam_last_acq, lam_last_trade, lam_end, acq_now, income,
            gain_rate, token_cost, prev_disc, star_w, slot_pen, second_w, loge_round, nob_sq, medal_val,
            special_val, joker_extra, deny, alt_frac, rho, acc_min, trade_min, buy_min, max_offers, swaps,
            new_prog, podium_val, deny_noble, invest_min, se_lock, learn
        )
    }
}

pub struct MaestroAgent {
    params: Params,
    tables: Option<Tables>,
    pub(crate) mem: trade::TradeMem,
}

impl MaestroAgent {
    pub fn new(seed: u64) -> Self {
        Self::with_params(seed, Params::default())
    }

    pub fn with_params(_seed: u64, params: Params) -> Self {
        Self {
            params,
            tables: None,
            mem: trade::TradeMem::default(),
        }
    }
}

/// A move that is always legal for the phase, used if a choice fails.
fn fallback(legal: &[Action]) -> Action {
    const SAFE: [Action; 6] = [
        Action::PassBid,
        Action::PassInitiative,
        Action::EndInvesting,
        Action::EndTrading,
        Action::RejectTrade,
        Action::SkipEvent,
    ];
    SAFE.iter()
        .find(|a| legal.contains(a))
        .cloned()
        .unwrap_or_else(|| legal[0].clone())
}

impl Agent for MaestroAgent {
    fn name(&self) -> String {
        "maestro".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
        if legal.is_empty() {
            return Action::EndTrading;
        }
        if self.tables.is_none() {
            self.tables = Some(Tables::new(game));
        }
        let tables = self.tables.take().expect("built above");
        let params = self.params.clone();
        let c = Ctx::new(game, &tables, &params);
        let me = me.0;
        let action = match game.phase() {
            Phase::Investing { remaining, .. } => phases::invest(&c, me, legal, *remaining),
            Phase::Acquiring(acq) => phases::acquire(&c, me, legal, acq),
            Phase::Trading { offer: None, .. } => self.propose(&c, me),
            Phase::Trading { offer: Some(o), .. } => self.respond(&c, me, legal, o),
            Phase::Producing(pr) => phases::produce(&c, me, legal, pr),
            Phase::Closing(_) => phases::close(&c, me, legal),
            Phase::GameOver => legal[0].clone(),
        };
        self.tables = Some(tables);
        let ok = match &action {
            Action::ProposeTrade { .. } | Action::CounterTrade { .. } => game.is_legal(&action),
            a => legal.contains(a),
        };
        if ok {
            action
        } else {
            self.mem.pending = None;
            fallback(legal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colosseum_engine::GameConfig;
    use std::sync::Arc;

    #[test]
    fn plays_full_games_legally() {
        let config = Arc::new(GameConfig::embedded());
        for n in 3..=5usize {
            for seed in 0..8u64 {
                let mut game = Game::new(config.clone(), n, seed).unwrap();
                let mut agents: Vec<MaestroAgent> = (0..n).map(|i| MaestroAgent::new(seed + i as u64)).collect();
                let mut steps = 0;
                while let Some(p) = game.current_player() {
                    let legal = game.legal_actions();
                    let a = agents[p.0].choose_action(&game, &legal);
                    game.apply(a).expect("maestro chose an illegal action");
                    steps += 1;
                    assert!(steps < 50_000, "game did not terminate");
                }
                assert!(game.is_over());
            }
        }
    }

    #[test]
    fn plays_full_mixed_games_legally() {
        let config = Arc::new(GameConfig::embedded());
        for n in 3..=5usize {
            for seed in 0..6u64 {
                let mut game = Game::new(config.clone(), n, 100 + seed).unwrap();
                let mut agents: Vec<Box<dyn Agent>> = (0..n)
                    .map(|i| -> Box<dyn Agent> {
                        let s = seed * 10 + i as u64;
                        match (i + seed as usize) % 4 {
                            0 => Box::new(MaestroAgent::new(s)),
                            1 => Box::new(colosseum_bot_planner::PlannerAgent::new(s)),
                            2 => Box::new(colosseum_bot_auctioneer::AuctioneerAgent::new(s)),
                            _ => Box::new(colosseum_bot_trader::TraderAgent::new(s)),
                        }
                    })
                    .collect();
                let mut steps = 0;
                while let Some(p) = game.current_player() {
                    let legal = game.legal_actions();
                    let a = agents[p.0].choose_action(&game, &legal);
                    game.apply(a).expect("illegal action");
                    steps += 1;
                    assert!(steps < 50_000, "game did not terminate");
                }
                assert!(game.is_over());
            }
        }
    }
}
