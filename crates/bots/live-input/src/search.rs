//! Monte Carlo search over the live-input heuristic, off by default
//! (`budget: 0`): against house bots it wins 69% of 4-player games, but
//! against any other kind of opponent its playouts mislead it and it does
//! worse than the heuristic alone. At investing, auction and trade decisions, a few candidates (always including the heuristic's
//! own choice) are each played out on determinized copies of the game, and
//! successive halving keeps the best. Playouts put the heuristic in my seat
//! and `rollout` bots in the others.

use crate::core::{Core, last_round};
use crate::params::Params;
use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Action, Game, Phase, PlayerId};

#[derive(Clone, Debug)]
pub struct SearchConfig {
    /// Bot for the other seats in playouts: "house" or "core".
    pub rollout: String,
    /// Playouts per searched decision, summed over candidates.
    pub budget: usize,
    /// Budget multiplier in rounds before the final one.
    pub early_boost: f64,
    pub max_candidates: usize,
    pub base_bonus: f64,
    pub threads: usize,
    /// Weight of the score margin in a playout's outcome.
    pub margin_weight: f64,
    /// Decisions to search: I = investing, A = auctions, P = proposing
    /// trades, R = answering offers. Others follow the heuristic.
    pub phases: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            rollout: "house".into(),
            // Off by default: search only helped when the real opponents
            // played like the rollout bots (see tuning/live-input/RESULTS.md).
            budget: 0,
            early_boost: 1.0,
            max_candidates: 6,
            base_bonus: 0.03,
            threads: 1,
            margin_weight: 0.15,
            phases: "IAPR".into(),
        }
    }
}

fn rollout_bot(name: &str, seed: u64, p: &Params) -> Box<dyn Agent + Send> {
    match name {
        "core" => Box::new(crate::CoreAgent::with_params(seed, p.clone())),
        // A different strong bot per seat, so the search isn't tuned to one
        // style of opponent.
        "mixed" => match seed % 4 {
            0 => Box::new(colosseum_bot_house::HouseAgent::new(seed)),
            1 => Box::new(colosseum_bot_oracle::OracleAgent::new(seed)),
            2 => Box::new(colosseum_bot_maestro::MaestroAgent::new(seed)),
            _ => Box::new(colosseum_bot_chameleon::ChameleonAgent::new(seed)),
        },
        _ => Box::new(colosseum_bot_house::HouseAgent::new(seed)),
    }
}

pub struct LiveInputAgent {
    cfg: SearchConfig,
    rng: Rng,
    base: Core,
}

impl LiveInputAgent {
    pub fn new(seed: u64) -> Self {
        Self::with_config(seed, Params::default(), SearchConfig::default())
    }

    pub fn with_config(seed: u64, p: Params, cfg: SearchConfig) -> Self {
        LiveInputAgent { cfg, rng: Rng::new(seed ^ 0x11FE_1290_0000_0001), base: Core::new(seed, p) }
    }
}

fn playout(game: &mut Game, me: PlayerId, p: &Params, others: &str, seed: u64) {
    let mut agents: Vec<Box<dyn Agent + Send>> = (0..game.num_players())
        .map(|i| {
            let s = seed.wrapping_add(i as u64 * 7919);
            if i == me.0 { Box::new(crate::CoreAgent::with_params(s, p.clone())) as Box<dyn Agent + Send> } else { rollout_bot(others, s, p) }
        })
        .collect();
    let mut steps = 0;
    while let Some(q) = game.current_player() {
        let legal = game.legal_actions();
        let a = agents[q.0].choose_action(game, &legal);
        if game.apply(a).is_err() || steps > 20_000 {
            break;
        }
        steps += 1;
    }
}

fn outcome(game: &Game, me: PlayerId, margin_w: f64) -> f64 {
    let ranking = game.ranking();
    let pos = ranking.iter().position(|&q| q == me).unwrap_or(0);
    let mine = game.player(me).best_score as f64;
    let best_other = game.players().iter().filter(|q| q.id != me).map(|q| q.best_score as f64).fold(0.0, f64::max);
    let win = if pos == 0 { 1.0 } else { 0.0 };
    let rank = 1.0 - pos as f64 / (game.num_players().max(2) - 1) as f64;
    let margin = ((mine - best_other) / 20.0).clamp(-1.0, 1.0);
    win + 0.25 * rank + margin_w * margin
}

impl LiveInputAgent {
    fn sample(&self, game: &Game, me: PlayerId, cands: &[Action], alive: &[usize], (world, seed): (u64, u64)) -> Vec<f64> {
        let world = game.determinized(world);
        alive
            .iter()
            .map(|&c| {
                let mut g = world.clone();
                if g.apply(cands[c].clone()).is_err() {
                    return -10.0;
                }
                playout(&mut g, me, &self.base.p, &self.cfg.rollout, seed);
                outcome(&g, me, self.cfg.margin_weight)
            })
            .collect()
    }

    fn search(&mut self, game: &Game, me: PlayerId, cands: Vec<Action>) -> Action {
        let k = cands.len();
        let rounds = (usize::BITS - (k - 1).leading_zeros()) as usize;
        let boost = if last_round(game) { 1.0 } else { self.cfg.early_boost };
        let per_round = ((self.cfg.budget as f64 * boost) as usize) / rounds.max(1);
        let mut sum = vec![0.0f64; k];
        let mut n = vec![0u32; k];
        let mut alive: Vec<usize> = (0..k).collect();
        while alive.len() > 1 {
            let samples = (per_round / alive.len()).max(2);
            let seeds: Vec<(u64, u64)> = (0..samples).map(|_| (self.rng.next_u64(), self.rng.next_u64())).collect();
            let threads = self.cfg.threads.clamp(1, seeds.len());
            let rows: Vec<Vec<f64>> = if threads == 1 {
                seeds.iter().map(|&s| self.sample(game, me, &cands, &alive, s)).collect()
            } else {
                let chunk = seeds.len().div_ceil(threads);
                let this = &*self;
                let (cands, alive) = (&cands, &alive);
                std::thread::scope(|scope| {
                    let hs: Vec<_> = seeds
                        .chunks(chunk)
                        .map(|part| scope.spawn(move || part.iter().map(|&s| this.sample(game, me, cands, alive, s)).collect::<Vec<_>>()))
                        .collect();
                    hs.into_iter().flat_map(|h| h.join().expect("search thread")).collect()
                })
            };
            for row in rows {
                for (&c, v) in alive.iter().zip(row) {
                    sum[c] += v;
                    n[c] += 1;
                }
            }
            let score = |c: usize| sum[c] / n[c].max(1) as f64 + if c == 0 { self.cfg.base_bonus } else { 0.0 };
            alive.sort_by(|&a, &b| score(b).total_cmp(&score(a)));
            alive.truncate(alive.len().div_ceil(2));
        }
        cands[alive[0]].clone()
    }

    fn candidates(&self, game: &Game, legal: &[Action], base: &Action) -> Option<Vec<Action>> {
        let mut c = vec![base.clone()];
        let mut add = |a: Action| {
            if !c.contains(&a) && legal.contains(&a) {
                c.push(a);
            }
        };
        let phase_key = match game.phase() {
            Phase::Investing { .. } => 'I',
            Phase::Acquiring(_) => 'A',
            Phase::Trading { offer: None, .. } => 'P',
            Phase::Trading { .. } => 'R',
            _ => return None,
        };
        if !self.cfg.phases.contains(phase_key) {
            return None;
        }
        match game.phase() {
            Phase::Investing { .. } => {
                add(Action::EndInvesting);
                for a in legal {
                    if matches!(a, Action::ExpandArena { .. } | Action::BuildLoge | Action::BuySeasonTicket) {
                        add(a.clone());
                    }
                }
                let mut buys: Vec<(u32, &Action)> = legal
                    .iter()
                    .filter_map(|a| match a {
                        Action::BuyProgram { program } => game.config().event(*program).map(|e| (e.base_score, a)),
                        _ => None,
                    })
                    .collect();
                buys.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
                for (_, a) in buys.into_iter().take(3) {
                    add(a.clone());
                }
            }
            Phase::Acquiring(acq) => match &acq.auction {
                None => {
                    add(Action::PassInitiative);
                    let min = game.rules().acquiring.minimum_opening_bid;
                    let mut seen = Vec::new();
                    for (m, tokens) in game.markets().iter().enumerate() {
                        let mut key = tokens.clone();
                        key.sort();
                        if !seen.contains(&key) {
                            seen.push(key);
                            add(Action::StartAuction { market: m, bid: min });
                        }
                    }
                }
                Some(au) => {
                    add(Action::PassBid);
                    for inc in [1, 3, 6] {
                        add(Action::Bid { amount: au.high_bid + inc });
                    }
                }
            },
            Phase::Trading { offer: None, .. } => {
                if *base == Action::EndTrading {
                    return None;
                }
                add(Action::EndTrading);
            }
            Phase::Trading { .. } => {
                add(Action::AcceptTrade);
                add(Action::RejectTrade);
            }
            _ => return None,
        }
        c.truncate(self.cfg.max_candidates);
        (c.len() > 1).then_some(c)
    }
}

impl Agent for LiveInputAgent {
    fn name(&self) -> String {
        "live-input".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let base = self.base.choose(game, legal);
        if legal.len() == 1 || self.cfg.budget == 0 {
            return base;
        }
        let Some(me) = game.current_player() else { return base };
        let Some(cands) = self.candidates(game, legal, &base) else { return base };
        let chosen = self.search(game, me, cands);
        if chosen != base {
            // Keep the heuristic's offer bookkeeping honest: it thinks it
            // made `base`, so forget that.
            self.base.forget_pending();
        }
        chosen
    }
}
