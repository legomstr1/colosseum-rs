//! Computer players. An agent sees the game and the legal actions and
//! returns one of them; it never has to check rules itself.

use crate::action::Action;
use crate::game::{Game, Phase};
use crate::rng::Rng;
use crate::types::{Asset, Assets, PlayerId};
use std::mem::discriminant;

pub trait Agent {
    fn name(&self) -> String;
    /// Picks one of `legal` (never empty) for the current player.
    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action;
}

/// Picks a random kind of action, then a random action of that kind, so
/// that long lists (like every possible bid) don't drown out the rest.
pub struct RandomAgent {
    rng: Rng,
}

impl RandomAgent {
    pub fn new(seed: u64) -> Self {
        Self { rng: Rng::new(seed) }
    }
}

impl Agent for RandomAgent {
    fn name(&self) -> String {
        "random".into()
    }

    fn choose_action(&mut self, _game: &Game, legal: &[Action]) -> Action {
        let mut kinds = Vec::new();
        for a in legal {
            if !kinds.contains(&discriminant(a)) {
                kinds.push(discriminant(a));
            }
        }
        let kind = kinds[self.rng.below(kinds.len())];
        let of_kind: Vec<&Action> = legal.iter().filter(|a| discriminant(*a) == kind).collect();
        of_kind[self.rng.below(of_kind.len())].clone()
    }
}

/// A simple heuristic player: chases its best program's assets, moves
/// nobles into its arena, and produces the highest-scoring event.
pub struct GreedyAgent {
    rng: Rng,
}

impl GreedyAgent {
    pub fn new(seed: u64) -> Self {
        Self { rng: Rng::new(seed) }
    }
}

/// The owned program with the highest base score.
fn target_program(game: &Game, me: PlayerId) -> Option<u8> {
    game.player(me)
        .programs
        .iter()
        .filter_map(|p| game.config().event(p.id))
        .max_by_key(|e| e.base_score)
        .map(|e| e.id)
}

/// Assets still missing for the target program.
fn deficit(game: &Game, me: PlayerId) -> Assets {
    let mut out = Assets::new();
    if let Some(def) = target_program(game, me).and_then(|id| game.config().event(id)) {
        let have = &game.player(me).assets;
        for (a, n) in def.requirement_counts().iter() {
            out.add(a, n.saturating_sub(have.count(a)));
        }
    }
    out
}

/// Rough worth of a set of tokens, in "useful tokens".
fn token_value(tokens: &[Asset], need: &Assets) -> f64 {
    let mut need = need.clone();
    tokens
        .iter()
        .map(|&a| {
            if need.remove(a, 1) {
                1.0
            } else if a == Asset::Joker {
                0.8
            } else if matches!(a, Asset::Emperor | Asset::SpecialEvent) {
                0.5
            } else {
                0.2
            }
        })
        .sum()
}

fn best_event(game: &Game, me: PlayerId) -> u32 {
    game.player(me)
        .programs
        .iter()
        .filter_map(|p| game.event_preview(me, p.id, 0))
        .map(|pv| pv.breakdown.total())
        .max()
        .unwrap_or(0)
}

impl GreedyAgent {
    fn pick_best(&mut self, scored: Vec<(f64, Action)>) -> Option<Action> {
        let best = scored.iter().map(|(s, _)| *s).fold(f64::NEG_INFINITY, f64::max);
        let top: Vec<Action> = scored
            .into_iter()
            .filter(|(s, _)| *s == best)
            .map(|(_, a)| a)
            .collect();
        self.rng.choose(&top).cloned()
    }

    fn invest(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let player = game.player(me);
        let rules = game.rules();
        let current_best = target_program(game, me)
            .and_then(|id| game.config().event(id))
            .map_or(0, |e| e.base_score);
        let needed_squares = target_program(game, me)
            .and_then(|id| game.config().event(id))
            .and_then(|e| rules.arena.squares_required_by_event_size.get(e.size as usize).copied())
            .unwrap_or(0);

        let buy = legal
            .iter()
            .filter_map(|a| match a {
                Action::BuyProgram { program } => game.config().event(*program).map(|e| (e, a)),
                _ => None,
            })
            .filter(|(e, _)| e.base_score > current_best + 5 && player.coins >= e.cost + 15)
            .max_by_key(|(e, _)| e.base_score);
        if let Some((_, a)) = buy {
            return a.clone();
        }
        if player.arena.len < needed_squares
            && let Some(a) = legal.iter().find(|a| matches!(a, Action::ExpandArena { .. }))
        {
            return a.clone();
        }
        if legal.contains(&Action::BuildLoge) {
            return Action::BuildLoge;
        }
        if legal.contains(&Action::BuySeasonTicket) && player.coins >= 25 {
            return Action::BuySeasonTicket;
        }
        Action::EndInvesting
    }

    fn acquire(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Acquiring(acq) = &game.phase() else {
            unreachable!()
        };
        let need = deficit(game, me);
        let coins = game.player(me).coins;
        match &acq.auction {
            None => {
                let min = game.rules().acquiring.minimum_opening_bid;
                let best = (0..game.markets().len())
                    .filter(|m| legal.contains(&Action::StartAuction { market: *m, bid: min }))
                    .map(|m| (token_value(&game.markets()[m], &need), m))
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                match best {
                    Some((value, market)) if value >= 1.0 => Action::StartAuction { market, bid: min },
                    _ => Action::PassInitiative,
                }
            }
            Some(auction) => {
                let value = token_value(&game.markets()[auction.market], &need);
                let max_pay = ((value * 6.0) as u32).min(coins.saturating_sub(5));
                let bid = Action::Bid {
                    amount: auction.high_bid + 1,
                };
                if auction.high_bid < max_pay && legal.contains(&bid) {
                    bid
                } else {
                    Action::PassBid
                }
            }
        }
    }

    fn respond_to_offer(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Trading { offer: Some(offer), .. } = &game.phase() else {
            return Action::EndTrading;
        };
        if !legal.contains(&Action::AcceptTrade) {
            return Action::RejectTrade;
        }
        let need = deficit(game, me);
        let gained = token_value(&offer.terms.give.to_vec(), &need);
        let mut needed_after = need.clone();
        // Giving away a token I need makes the deficit grow.
        let lost: f64 = offer
            .terms
            .receive
            .to_vec()
            .iter()
            .map(|&a| {
                let have = game.player(me).assets.count(a);
                let required = target_program(game, me)
                    .and_then(|id| game.config().event(id))
                    .map_or(0, |e| e.requirement_counts().count(a));
                if have <= required {
                    needed_after.add(a, 1);
                    1.0
                } else {
                    0.2
                }
            })
            .sum();
        let coins = offer.terms.give_coins as f64 - offer.terms.receive_coins as f64;
        if (gained - lost) * 6.0 + coins > 0.0 {
            Action::AcceptTrade
        } else {
            Action::RejectTrade
        }
    }

    fn produce(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let dice_moves: Vec<&Action> = legal
            .iter()
            .filter(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }))
            .collect();
        if !dice_moves.is_empty() {
            let scored = dice_moves
                .into_iter()
                .map(|a| {
                    let mut next = game.clone();
                    next.apply(a.clone()).expect("legal move");
                    let medals = next.player(me).medals as f64;
                    (best_event(&next, me) as f64 + medals * 2.0, a.clone())
                })
                .collect();
            return self.pick_best(scored).expect("non-empty");
        }

        let last_round = game.round() >= game.rules().num_rounds;
        let medals = if last_round { game.player(me).medals } else { 0 };
        let produce = legal
            .iter()
            .filter_map(|a| match a {
                Action::ProduceEvent { program, medals: m } if *m == medals => game
                    .event_preview(me, *program, *m)
                    .map(|pv| (pv.breakdown.total() as f64, a.clone())),
                _ => None,
            })
            .collect();
        self.pick_best(produce).unwrap_or(Action::SkipEvent)
    }

    fn close(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let need = deficit(game, me);
        let have = &game.player(me).assets;
        let scored = legal
            .iter()
            .filter_map(|a| match a {
                // Discard what we hold the most spare copies of.
                Action::DiscardAsset { asset } => {
                    Some((have.count(*asset) as f64 - need.count(*asset) as f64 * 10.0, a.clone()))
                }
                // Ask for what we lack most.
                Action::RequestAsset { asset } => Some((need.count(*asset) as f64, a.clone())),
                _ => None,
            })
            .collect();
        self.pick_best(scored).unwrap_or_else(|| legal[0].clone())
    }
}

impl Agent for GreedyAgent {
    fn name(&self) -> String {
        "greedy".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let me = game.current_player().expect("game in progress");
        match &game.phase() {
            Phase::Investing { .. } => self.invest(game, me, legal),
            Phase::Acquiring(_) => self.acquire(game, me, legal),
            Phase::Trading { offer: None, .. } => Action::EndTrading,
            Phase::Trading { .. } => self.respond_to_offer(game, me, legal),
            Phase::Producing(_) => self.produce(game, me, legal),
            Phase::Closing(_) => self.close(game, me, legal),
            Phase::GameOver => unreachable!("no decisions after the game ends"),
        }
    }
}

/// Plays a game to the end with one agent per seat.
pub fn play_out(game: &mut Game, agents: &mut [Box<dyn Agent>]) {
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let action = agents[p.0].choose_action(game, &legal);
        game.apply(action).expect("agents choose legal actions");
    }
}
