//! Monte Carlo rollout bot.
//!
//! * Forced moves are returned immediately; the closing phase and trade
//!   responses are delegated to the heuristic `GreedyAgent`.
//! * The producing phase is solved exactly: every sequence of noble moves
//!   the dice allow is tried and the one leading to the best event wins.
//! * Investing and acquiring decisions are searched: the candidate list is
//!   pruned, then each candidate is applied to several *determinized*
//!   copies of the game (hidden bag order and dice RNG re-drawn) and played
//!   to the end with greedy players. The candidate with the best average
//!   outcome (win share plus a small score-margin bonus) is chosen.
//!
//! Everything is a function of the seed and the position, so results are
//! reproducible.

use colosseum_engine::agent::{Agent, GreedyAgent};
use colosseum_engine::rng::Rng;
use std::collections::HashMap;
use colosseum_engine::{Action, Game, Phase, PlayerId};

/// Default number of playouts spent on a searched decision.
pub const DEFAULT_ROLLOUTS: usize = 24;

/// Searched decisions look at no more candidates than this (best-first).
const MAX_CANDIDATES: usize = 7;

pub struct RolloutAgent {
    rng: Rng,
    greedy: GreedyAgent,
    rollouts: usize,
}

impl RolloutAgent {
    pub fn new(seed: u64) -> Self {
        Self::with_budget(seed, DEFAULT_ROLLOUTS)
    }

    /// `rollouts` is the playout budget per searched decision.
    pub fn with_budget(seed: u64, rollouts: usize) -> Self {
        Self {
            rng: Rng::new(seed ^ 0xA5A5_5A5A_1234_5678),
            greedy: GreedyAgent::new(seed.wrapping_add(17)),
            rollouts: rollouts.max(1),
        }
    }
}

/// Copy of `game` with the hidden information (bag order, green pool order)
/// and the dice RNG re-drawn.
fn determinize(game: &Game, rng: &mut Rng) -> Game {
    let Ok(mut v) = serde_json::to_value(game) else {
        return game.clone();
    };
    if let Some(obj) = v.as_object_mut() {
        for key in ["bag", "green_pool"] {
            if let Some(arr) = obj.get_mut(key).and_then(|a| a.as_array_mut()) {
                rng.shuffle(arr);
            }
        }
        // The history is not needed for rollouts; dropping it keeps copies cheap.
        obj.insert("history".into(), serde_json::Value::Array(Vec::new()));
        // Keep the state below 2^53 so it survives JSON numbers.
        let state = rng.next_u64() >> 12;
        obj.insert("rng".into(), serde_json::json!({ "state": state }));
    }
    serde_json::from_value(v).unwrap_or_else(|_| game.clone())
}

fn playout(game: &mut Game, seed: u64) {
    let n = game.num_players();
    let mut agents: Vec<GreedyAgent> = (0..n)
        .map(|i| GreedyAgent::new(seed.wrapping_add(i as u64 * 7919)))
        .collect();
    let mut guard = 0;
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        if legal.is_empty() {
            break;
        }
        let a = agents[p.0].choose_action(game, &legal);
        if game.apply(a).is_err() {
            break;
        }
        guard += 1;
        if guard > 20_000 {
            break;
        }
    }
}

/// Outcome for `me` in a finished (or cut-off) game: mostly "did I win",
/// with small rank and score-margin terms to reduce variance.
fn evaluate(game: &Game, me: PlayerId) -> f64 {
    let ranking = game.ranking();
    let pos = ranking.iter().position(|&p| p == me).unwrap_or(0);
    let mine = game.player(me);
    let win = if pos == 0 { 1.0 } else { 0.0 };
    let best_other = game
        .players()
        .iter()
        .filter(|p| p.id != me)
        .map(|p| p.best_score as f64)
        .fold(0.0, f64::max);
    let margin = ((mine.best_score as f64 - best_other) / 20.0).clamp(-1.0, 1.0);
    let rank_share = 1.0 - pos as f64 / (game.num_players().max(2) - 1) as f64;
    win + 0.25 * rank_share + 0.15 * margin
}

fn best_event_value(game: &Game, me: PlayerId, medals: u32) -> f64 {
    game.player(me)
        .programs
        .iter()
        .filter_map(|p| game.event_preview(me, p.id, medals))
        .map(|pv| pv.breakdown.total() as f64)
        .fold(0.0, f64::max)
}

fn is_dice_move(a: &Action) -> bool {
    matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. })
}

impl RolloutAgent {
    /// Evaluates each candidate on shared determinizations; returns the best.
    fn search(&mut self, game: &Game, me: PlayerId, cands: Vec<Action>) -> Action {
        if cands.len() == 1 {
            return cands[0].clone();
        }
        let mut cands = cands;
        cands.truncate(MAX_CANDIDATES);
        // Playouts get slower with more players, so scale the budget down.
        let budget = self.rollouts * 4 / game.num_players().max(1);
        let dets = (budget / cands.len()).clamp(2, 12);
        let mut totals = vec![0.0f64; cands.len()];
        for _ in 0..dets {
            let base = determinize(game, &mut self.rng);
            let seed = self.rng.next_u64();
            for (i, a) in cands.iter().enumerate() {
                let mut g = base.clone();
                if g.apply(a.clone()).is_err() {
                    continue;
                }
                playout(&mut g, seed);
                totals[i] += evaluate(&g, me);
            }
        }
        let best = (0..cands.len())
            .max_by(|&a, &b| totals[a].total_cmp(&totals[b]))
            .unwrap_or(0);
        cands[best].clone()
    }

    fn invest(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let mut buys: Vec<(u32, Action)> = legal
            .iter()
            .filter_map(|a| match a {
                Action::BuyProgram { program } => game.config().event(*program).map(|e| (e.base_score, a.clone())),
                _ => None,
            })
            .collect();
        buys.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
        // Doing nothing always stays in the candidate list.
        let mut cands: Vec<Action> = vec![Action::EndInvesting];
        cands.extend(buys.into_iter().take(3).map(|(_, a)| a));
        let exchange = |a: &&Action| matches!(a, Action::ExchangeMedalForCoins | Action::ExchangeSpecialEventForMedal);
        cands.extend(legal.iter().filter(|a| !matches!(a, Action::BuyProgram { .. } | Action::EndInvesting) && !exchange(a)).cloned());
        cands.extend(legal.iter().filter(exchange).cloned());
        self.search(game, me, cands)
    }

    fn acquire(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let mut cands = Vec::new();
        let Phase::Acquiring(acq) = game.phase() else {
            return legal[0].clone();
        };
        match &acq.auction {
            None => {
                let min = game.rules().acquiring.minimum_opening_bid;
                let mut seen: Vec<Vec<_>> = Vec::new();
                for (m, tokens) in game.markets().iter().enumerate() {
                    let mut key = tokens.clone();
                    key.sort();
                    if seen.contains(&key) {
                        continue;
                    }
                    seen.push(key);
                    let a = Action::StartAuction { market: m, bid: min };
                    if legal.contains(&a) {
                        cands.push(a);
                    }
                }
                cands.push(Action::PassInitiative);
            }
            Some(au) => {
                for inc in [1, 3, 6] {
                    let a = Action::Bid { amount: au.high_bid + inc };
                    if legal.contains(&a) {
                        cands.push(a);
                    }
                }
                cands.push(Action::PassBid);
            }
        }
        cands.retain(|a| legal.contains(a));
        if cands.is_empty() {
            return legal[0].clone();
        }
        self.search(game, me, cands)
    }

    /// Exhaustive search over the noble moves the dice allow.
    fn produce(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let moves: Vec<&Action> = legal.iter().filter(|a| is_dice_move(a)).collect();
        if moves.is_empty() {
            return self.pick_produce(game, me, legal);
        }
        let mut best: Option<(f64, Action)> = None;
        let mut memo = Memo::new();
        for a in moves {
            let mut g = game.clone();
            if g.apply(a.clone()).is_err() {
                continue;
            }
            let v = dice_value(&g, me, 0, &mut memo);
            if best.as_ref().is_none_or(|(b, _)| v > *b) {
                best = Some((v, a.clone()));
            }
        }
        best.map_or_else(|| legal[0].clone(), |(_, a)| a)
    }

    fn pick_produce(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let last_round = game.round() >= game.rules().num_rounds;
        let medals = if last_round { game.player(me).medals } else { 0 };
        legal
            .iter()
            .filter_map(|a| match a {
                Action::ProduceEvent { program, medals: m } if *m == medals => game
                    .event_preview(me, *program, *m)
                    .map(|pv| (pv.breakdown.total(), a.clone())),
                _ => None,
            })
            .max_by_key(|(s, _)| *s)
            .map(|(_, a)| a)
            .unwrap_or(Action::SkipEvent)
    }
}

type Memo = HashMap<(Vec<u8>, Vec<bool>, Vec<usize>, u32), f64>;

fn dice_value(game: &Game, me: PlayerId, depth: u32, memo: &mut Memo) -> f64 {
    let key = match game.phase() {
        Phase::Producing(pr) => {
            let mut moved = pr.moved.clone();
            moved.sort_unstable();
            Some((
                game.nobles().iter().map(|n| n.position).collect::<Vec<u8>>(),
                pr.used.clone(),
                moved,
                game.player(me).medals,
            ))
        }
        _ => None,
    };
    if let Some(v) = key.as_ref().and_then(|k| memo.get(k)) {
        return *v;
    }
    let v = dice_value_uncached(game, me, depth, memo);
    if let Some(k) = key {
        memo.insert(k, v);
    }
    v
}

fn dice_value_uncached(game: &Game, me: PlayerId, depth: u32, memo: &mut Memo) -> f64 {
    let legal = game.legal_actions();
    let moves: Vec<&Action> = legal.iter().filter(|a| is_dice_move(a)).collect();
    if moves.is_empty() || depth > 4 {
        let opp: f64 = game
            .players()
            .iter()
            .filter(|q| q.id != me)
            .map(|q| {
                game.nobles_in_arena(q.id)
                    .into_iter()
                    .map(|i| game.rules().noble_value(game.nobles()[i].kind) as f64)
                    .sum::<f64>()
            })
            .sum();
        return best_event_value(game, me, 0) + 3.0 * game.player(me).medals as f64 - 0.3 * opp;
    }
    let mut best = f64::NEG_INFINITY;
    for a in moves {
        let mut g = game.clone();
        if g.apply(a.clone()).is_ok() {
            best = best.max(dice_value(&g, me, depth + 1, memo));
        }
    }
    best
}

impl Agent for RolloutAgent {
    fn name(&self) -> String {
        "rollout".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        if legal.len() == 1 {
            return legal[0].clone();
        }
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
        match game.phase() {
            Phase::Investing { .. } => self.invest(game, me, legal),
            Phase::Acquiring(_) => self.acquire(game, me, legal),
            Phase::Producing(_) => self.produce(game, me, legal),
            _ => self.greedy.choose_action(game, legal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colosseum_engine::GameConfig;
    use colosseum_engine::agent::play_out;
    use std::sync::Arc;

    #[test]
    fn plays_legal_games() {
        let config = Arc::new(GameConfig::embedded());
        for n in 3..=5 {
            let mut game = Game::new(config.clone(), n, 7 + n as u64).unwrap();
            let mut agents: Vec<Box<dyn Agent>> = (0..n)
                .map(|i| -> Box<dyn Agent> {
                    if i == 0 {
                        Box::new(RolloutAgent::with_budget(1, 8))
                    } else {
                        Box::new(GreedyAgent::new(i as u64))
                    }
                })
                .collect();
            play_out(&mut game, &mut agents);
            assert!(game.is_over());
        }
    }
}
