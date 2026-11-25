//! Rebuilds a game's action history from its seed and its log text.
//!
//! Servers before the save endpoint kept no way to export a game, only its
//! view, whose log holds every event as text. Since the engine is
//! deterministic, searching for the actions whose events print the same
//! lines recovers the history. Trade offers aren't listed as legal actions,
//! so their terms are parsed back from the "offers" lines.

use colosseum_engine::{Action, Asset, Assets, Game, Phase, PlayerId, TradeTerms};

/// Most game states the search may visit before giving up.
const BUDGET: usize = 2_000_000;

pub struct Recovered {
    pub history: Vec<Action>,
}

/// `log` is the event text after the "New game" line. `human` says which
/// seats are humans: the old server stopped only when one of them was to act.
pub fn recover(game: Game, log: &[String], human: &[bool]) -> Result<Recovered, String> {
    let mut search = Search { log, human, visited: 0, furthest: 0 };
    let mut history = Vec::new();
    if search.dfs(game, 0, &mut history) {
        Ok(Recovered { history })
    } else {
        let at = search.furthest;
        Err(format!(
            "no actions reproduce the log past line {at} ({:?}); visited {} states",
            log.get(at).map_or("end", String::as_str),
            search.visited
        ))
    }
}

struct Search<'a> {
    log: &'a [String],
    human: &'a [bool],
    visited: usize,
    furthest: usize,
}

impl Search<'_> {
    fn dfs(&mut self, game: Game, at: usize, history: &mut Vec<Action>) -> bool {
        self.visited += 1;
        self.furthest = self.furthest.max(at);
        if self.visited > BUDGET {
            return false;
        }
        let Some(p) = game.current_player() else {
            return at == self.log.len();
        };
        if at == self.log.len() && self.human[p.0] {
            return true;
        }

        // Actions that print the next lines, then actions that print nothing.
        let mut printing = Vec::new();
        let mut silent = Vec::new();
        for action in self.candidates(&game, p, at) {
            let mut next = game.clone();
            let Ok(events) = next.apply(action.clone()) else {
                continue;
            };
            let lines: Vec<String> = events.iter().map(ToString::to_string).collect();
            if lines.is_empty() {
                silent.push((action, next));
            } else if self.log.get(at..at + lines.len()).is_some_and(|l| l == lines.as_slice()) {
                printing.push((action, next, lines.len()));
            }
        }
        for (action, next, n) in printing {
            history.push(action);
            if self.dfs(next, at + n, history) {
                return true;
            }
            history.pop();
        }
        for (action, next) in silent {
            history.push(action);
            if self.dfs(next, at, history) {
                return true;
            }
            history.pop();
        }
        false
    }

    fn candidates(&self, game: &Game, p: PlayerId, at: usize) -> Vec<Action> {
        let mut actions = game.legal_actions();
        if let (Phase::Trading { offer, .. }, Some(line)) = (game.phase(), self.log.get(at))
            && let Some((from, to, terms)) = parse_offer(line)
            && from == p
        {
            actions.push(if offer.is_some() {
                Action::CounterTrade { terms }
            } else {
                Action::ProposeTrade { to, terms }
            });
        }
        actions
    }
}

/// Parses "P1 offers P2: 1 Horse, 2 Joker + 3 coins for nothing + 0 coins".
fn parse_offer(line: &str) -> Option<(PlayerId, PlayerId, TradeTerms)> {
    let (from, rest) = line.split_once(" offers ")?;
    let (to, rest) = rest.split_once(": ")?;
    let (give, receive) = rest.split_once(" coins for ")?;
    let (give, give_coins) = give.rsplit_once(" + ")?;
    let receive = receive.strip_suffix(" coins")?;
    let (receive, receive_coins) = receive.rsplit_once(" + ")?;
    Some((
        parse_player(from)?,
        parse_player(to)?,
        TradeTerms {
            give: parse_assets(give)?,
            give_coins: give_coins.parse().ok()?,
            receive: parse_assets(receive)?,
            receive_coins: receive_coins.parse().ok()?,
        },
    ))
}

fn parse_player(s: &str) -> Option<PlayerId> {
    let n: usize = s.strip_prefix('P')?.parse().ok()?;
    n.checked_sub(1).map(PlayerId)
}

fn parse_assets(s: &str) -> Option<Assets> {
    let mut assets = Assets::new();
    if s == "nothing" {
        return Some(assets);
    }
    for part in s.split(", ") {
        let (n, name) = part.split_once(' ')?;
        let asset = Asset::ALL.into_iter().find(|a| a.name() == name)?;
        assets.add(asset, n.parse().ok()?);
    }
    Some(assets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use colosseum_engine::GameConfig;
    use colosseum_engine::agent::{Agent, GreedyAgent};
    use colosseum_engine::GameEvent;
    use std::sync::Arc;

    #[test]
    fn offer_text_round_trips() {
        let mut give = Assets::new();
        give.add(Asset::ALL[0], 2);
        give.add(Asset::ALL[3], 1);
        let terms = TradeTerms { give, give_coins: 3, receive: Assets::new(), receive_coins: 0 };
        let line = GameEvent::TradeProposed { from: PlayerId(0), to: PlayerId(2), terms: terms.clone() }.to_string();
        assert_eq!(parse_offer(&line), Some((PlayerId(0), PlayerId(2), terms)));
    }

    /// Plays a bot game partway, then recovers it from the log text alone.
    #[test]
    fn recovers_a_bot_game_from_its_log() {
        let config = Arc::new(GameConfig::embedded());
        let mut game = Game::new(config.clone(), 3, 7).unwrap();
        let mut bots: Vec<GreedyAgent> = (0..3).map(|i| GreedyAgent::new(i + 1)).collect();
        let mut log = Vec::new();
        for _ in 0..120 {
            let Some(p) = game.current_player() else { break };
            let legal = game.legal_actions();
            let a = bots[p.0].choose_action(&game, &legal);
            log.extend(game.apply(a).unwrap().iter().map(ToString::to_string));
        }
        // Treat whoever is to act now as the human.
        let mut human = vec![false; 3];
        human[game.current_player().unwrap().0] = true;
        let fresh = Game::new(config.clone(), 3, 7).unwrap();
        let r = recover(fresh, &log, &human).unwrap();
        let rebuilt = Game::replay(config, 3, 7, &r.history).unwrap();
        assert_eq!(rebuilt.players(), game.players());
        assert_eq!(rebuilt.phase(), game.phase());
        // Nobles of one kind on one square look alike in the log, so which
        // of them moved can differ: compare them as a set.
        let sorted = |g: &Game| {
            let mut n: Vec<String> = g.nobles().iter().map(|n| format!("{n:?}")).collect();
            n.sort();
            n
        };
        assert_eq!(sorted(&rebuilt), sorted(&game));
    }
}
