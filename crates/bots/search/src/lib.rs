//! Search: a base bot improved by Monte Carlo search.
//!
//! At each decision that shapes the game (investing, opening and bidding in
//! auctions, proposing and answering trades) a short list of candidate
//! actions is built, always including the base bot's own choice. Each
//! candidate is applied to many *determinized* copies of the game (bag
//! order, green pool order and future dice redrawn, so the search never
//! sees the future) and played to the end, with the base bot in my seat
//! and a fast rollout bot in the others. Candidates share determinizations and rollout seeds (common random
//! numbers), and successive halving drops the weaker half each round so the
//! budget goes to the close calls. Other decisions (noble moves, closing)
//! are left to the base bot.
//!
//! This is the search half of an AlphaZero-style player: later, a learned
//! value net can replace the playouts and a learned policy can rank the
//! candidates.

use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Action, Asset, Assets, Game, Phase, PlayerId, TradeTerms};

#[derive(Clone, Debug)]
pub struct Config {
    /// Bot whose choices seed the candidate list and who plays the
    /// decisions that aren't searched.
    pub base: String,
    /// Bot that plays the opponents' seats in playouts: the closer to how
    /// the real opponents play, the better (house vs house: 62% wins with
    /// house here, 32% with oracle).
    /// My own seat is played by the base bot, so playouts follow the plan
    /// the base bot will actually carry out.
    pub rollout: String,
    /// Playouts per searched decision (summed over candidates).
    pub budget: usize,
    /// Most candidates per decision.
    pub max_candidates: usize,
    /// Bonus added to the base bot's choice, so noise alone doesn't
    /// overrule it.
    pub base_bonus: f64,
    /// Threads per decision. Results don't depend on it. Keep 1 when many
    /// games already run in parallel (tournaments, training).
    pub threads: usize,
    /// Wider candidate lists: more bid levels and programs, and trade
    /// offers (coins or a spare token for a token I need) even when the
    /// base bot would stop trading.
    pub wide: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base: "house".into(),
            rollout: "house".into(),
            budget: 96,
            max_candidates: 6,
            base_bonus: 0.03,
            threads: 1,
            wide: false,
        }
    }
}

pub fn make_bot(name: &str, seed: u64) -> Box<dyn Agent + Send> {
    match name {
        "house" => Box::new(colosseum_bot_house::HouseAgent::new(seed)),
        "oracle" => Box::new(colosseum_bot_oracle::OracleAgent::new(seed)),
        "maestro" => Box::new(colosseum_bot_maestro::MaestroAgent::new(seed)),
        "chameleon" => Box::new(colosseum_bot_chameleon::ChameleonAgent::new(seed)),
        other => panic!("search: unknown bot '{other}'"),
    }
}

pub struct SearchAgent {
    cfg: Config,
    rng: Rng,
    base: Box<dyn Agent + Send>,
    decisions: Vec<Decision>,
}

/// One searched decision, for analysis: what the base bot wanted and what
/// the search chose, both described in words.
#[derive(Clone, Debug)]
pub struct Decision {
    pub round: u32,
    pub situation: &'static str,
    pub base: String,
    pub chosen: String,
}

/// Short description of `a` in `game` (bids relative to the high bid,
/// programs by base score).
pub fn describe(game: &Game, a: &Action) -> String {
    match a {
        Action::BuyProgram { program } => {
            format!("buy program (score {})", game.config().event(*program).map_or(0, |e| e.base_score))
        }
        Action::ExpandArena { .. } => "expand arena".into(),
        Action::BuildLoge => "build loge".into(),
        Action::BuySeasonTicket => "buy season ticket".into(),
        Action::EndInvesting => "end investing".into(),
        Action::StartAuction { .. } => "open an auction".into(),
        Action::PassInitiative => "pass initiative".into(),
        Action::Bid { amount } => match game.phase() {
            Phase::Acquiring(acq) => match &acq.auction {
                Some(au) => format!("bid +{}", amount.saturating_sub(au.high_bid)),
                None => "bid".into(),
            },
            _ => "bid".into(),
        },
        Action::PassBid => "pass bid".into(),
        Action::ProposeTrade { terms, .. } => {
            if terms.give_coins > 0 {
                "offer coins for a token".into()
            } else if terms.give.total() > 1 {
                "offer 2-for-1 swap".into()
            } else {
                "offer 1-for-1 swap".into()
            }
        }
        Action::EndTrading => "end trading".into(),
        Action::AcceptTrade => "accept".into(),
        Action::RejectTrade => "reject".into(),
        other => format!("{other:?}"),
    }
}

impl SearchAgent {
    pub fn new(seed: u64) -> Self {
        Self::with_config(seed, Config::default())
    }

    pub fn with_config(seed: u64, cfg: Config) -> Self {
        Self {
            base: make_bot(&cfg.base, seed),
            rng: Rng::new(seed ^ 0x5EA2_C400_0000_0001),
            cfg,
            decisions: Vec::new(),
        }
    }
}

/// Plays `game` to the end: `me` as `mine`, everyone else as `others`.
fn playout(game: &mut Game, me: PlayerId, mine: &str, others: &str, seed: u64) {
    let mut agents: Vec<Box<dyn Agent + Send>> = (0..game.num_players())
        .map(|i| make_bot(if i == me.0 { mine } else { others }, seed.wrapping_add(i as u64 * 7919)))
        .collect();
    let mut steps = 0;
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let a = agents[p.0].choose_action(game, &legal);
        if game.apply(a).is_err() || steps > 20_000 {
            break;
        }
        steps += 1;
    }
}

/// Outcome for `me`: mostly "did I win", plus small rank and score-margin
/// terms that cut variance.
fn outcome(game: &Game, me: PlayerId) -> f64 {
    let ranking = game.ranking();
    let pos = ranking.iter().position(|&p| p == me).unwrap_or(0);
    let mine = game.player(me).best_score as f64;
    let best_other = game
        .players()
        .iter()
        .filter(|p| p.id != me)
        .map(|p| p.best_score as f64)
        .fold(0.0, f64::max);
    let win = if pos == 0 { 1.0 } else { 0.0 };
    let rank = 1.0 - pos as f64 / (game.num_players().max(2) - 1) as f64;
    let margin = ((mine - best_other) / 20.0).clamp(-1.0, 1.0);
    win + 0.25 * rank + 0.15 * margin
}

/// Outcome of each `alive` candidate on one determinized world.
fn sample(game: &Game, me: PlayerId, cands: &[Action], alive: &[usize], (world, seed): (u64, u64), cfg: &Config) -> Vec<f64> {
    let world = game.determinized(world);
    alive
        .iter()
        .map(|&c| {
            let mut g = world.clone();
            if g.apply(cands[c].clone()).is_err() {
                return -10.0;
            }
            playout(&mut g, me, &cfg.base, &cfg.rollout, seed);
            outcome(&g, me)
        })
        .collect()
}

fn run_samples(game: &Game, me: PlayerId, cands: &[Action], alive: &[usize], seeds: &[(u64, u64)], cfg: &Config) -> Vec<Vec<f64>> {
    let threads = cfg.threads.clamp(1, seeds.len().max(1));
    if threads == 1 {
        return seeds.iter().map(|&s| sample(game, me, cands, alive, s, cfg)).collect();
    }
    let chunk = seeds.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = seeds
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(|&s| sample(game, me, cands, alive, s, cfg)).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("search thread")).collect()
    })
}

impl SearchAgent {
    /// Every searched decision so far.
    pub fn decisions(&self) -> &[Decision] {
        &self.decisions
    }

    /// Successive halving over `cands`; `cands[0]` is the base bot's choice.
    fn search(&mut self, game: &Game, me: PlayerId, cands: Vec<Action>) -> Action {
        if cands.len() == 1 {
            return cands[0].clone();
        }
        let k = cands.len();
        let rounds = (usize::BITS - (k - 1).leading_zeros()) as usize; // ceil(log2 k)
        let per_round = self.cfg.budget / rounds.max(1);
        let mut sum = vec![0.0f64; k];
        let mut n = vec![0u32; k];
        let mut alive: Vec<usize> = (0..k).collect();
        while alive.len() > 1 {
            let samples = (per_round / alive.len()).max(2);
            let seeds: Vec<(u64, u64)> = (0..samples).map(|_| (self.rng.next_u64(), self.rng.next_u64())).collect();
            // One row of outcomes per sample, summed in sample order so the
            // result is the same for any thread count.
            let rows = run_samples(game, me, &cands, &alive, &seeds, &self.cfg);
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

    /// Candidates for this decision, base choice first; `None` = don't search.
    fn candidates(&self, game: &Game, legal: &[Action], base: &Action) -> Option<Vec<Action>> {
        let mut c = vec![base.clone()];
        let mut add = |a: Action| {
            if !c.contains(&a) && legal.contains(&a) {
                c.push(a);
            }
        };
        match game.phase() {
            Phase::Investing { .. } => {
                add(Action::EndInvesting);
                for a in legal {
                    if matches!(a, Action::ExpandArena { .. } | Action::BuildLoge | Action::BuySeasonTicket) {
                        add(a.clone());
                    }
                }
                // The best programs on offer by base score.
                let mut buys: Vec<(u32, &Action)> = legal
                    .iter()
                    .filter_map(|a| match a {
                        Action::BuyProgram { program } => game.config().event(*program).map(|e| (e.base_score, a)),
                        _ => None,
                    })
                    .collect();
                buys.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
                for (_, a) in buys.into_iter().take(if self.cfg.wide { 5 } else { 3 }) {
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
                    let incs: &[u32] = if self.cfg.wide { &[1, 2, 4, 7, 11] } else { &[1, 3, 6] };
                    for &inc in incs {
                        add(Action::Bid { amount: au.high_bid + inc });
                    }
                }
            },
            Phase::Trading { offer: None, .. } => {
                if self.cfg.wide {
                    add(Action::EndTrading);
                    for a in trade_offers(game) {
                        if game.is_legal(&a) && !c.contains(&a) {
                            c.push(a);
                        }
                    }
                } else if *base == Action::EndTrading {
                    return None;
                } else {
                    add(Action::EndTrading);
                }
            }
            Phase::Trading { .. } => {
                add(Action::AcceptTrade);
                add(Action::RejectTrade);
            }
            _ => return None,
        }
        c.truncate(if self.cfg.wide { self.cfg.max_candidates + 2 } else { self.cfg.max_candidates });
        if c.len() == 1 {
            return None;
        }
        Some(c)
    }
}

/// Tokens still missing for each owned program, worst case per type.
fn needs(game: &Game, me: PlayerId) -> Assets {
    let have = &game.player(me).assets;
    let mut need = Assets::new();
    for p in game.player(me).programs.iter().filter(|p| !p.produced) {
        if let Some(def) = game.config().event(p.id) {
            for (a, n) in def.requirement_counts().iter() {
                let short = n.saturating_sub(have.count(a));
                if short > need.count(a) {
                    need.add(a, short - need.count(a));
                }
            }
        }
    }
    need
}

/// A few trade offers for the current player: for the two token types it
/// is shortest of, ask the rival holding most of them for one, paying 4 or
/// 9 coins, or a spare token of its own.
fn trade_offers(game: &Game) -> Vec<Action> {
    let Some(me) = game.current_player() else { return Vec::new() };
    let need = needs(game, me);
    let mine = &game.player(me).assets;
    let mut wanted: Vec<(u32, Asset)> = need.iter().filter(|(a, _)| a.is_tradeable()).map(|(a, n)| (n, a)).collect();
    wanted.sort_by(|a, b| b.0.cmp(&a.0));
    // The spare token I hold most of (not needed by any of my programs).
    let mut all_req = Assets::new();
    for p in game.player(me).programs.iter().filter(|p| !p.produced) {
        if let Some(def) = game.config().event(p.id) {
            for (a, n) in def.requirement_counts().iter() {
                if n > all_req.count(a) {
                    all_req.add(a, n - all_req.count(a));
                }
            }
        }
    }
    let spare = mine
        .iter()
        .filter(|(a, _)| a.is_tradeable() && *a != Asset::Joker)
        .map(|(a, n)| (n.saturating_sub(all_req.count(a)), a))
        .filter(|(n, _)| *n > 0)
        .max_by_key(|(n, _)| *n)
        .map(|(_, a)| a);
    let mut out = Vec::new();
    for &(_, want) in wanted.iter().take(2) {
        let Some(from) = game
            .players()
            .iter()
            .filter(|q| q.id != me && q.assets.count(want) > 0)
            .max_by_key(|q| q.assets.count(want))
            .map(|q| q.id)
        else {
            continue;
        };
        let get: Assets = [want].into_iter().collect();
        for coins in [4, 9] {
            out.push(Action::ProposeTrade {
                to: from,
                terms: TradeTerms { give: Assets::new(), give_coins: coins, receive: get.clone(), receive_coins: 0 },
            });
        }
        if let Some(give) = spare.filter(|&g| g != want) {
            out.push(Action::ProposeTrade {
                to: from,
                terms: TradeTerms { give: [give].into_iter().collect(), give_coins: 0, receive: get.clone(), receive_coins: 0 },
            });
        }
    }
    out
}

impl Agent for SearchAgent {
    fn name(&self) -> String {
        "search".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let base = self.base.choose_action(game, legal);
        if legal.len() == 1 {
            return base;
        }
        let Some(me) = game.current_player() else {
            return base;
        };
        let Some(cands) = self.candidates(game, legal, &base) else {
            return base;
        };
        let chosen = self.search(game, me, cands);
        let situation = match game.phase() {
            Phase::Investing { .. } => "investing",
            Phase::Acquiring(acq) if acq.auction.is_none() => "opening an auction",
            Phase::Acquiring(_) => "bidding",
            Phase::Trading { offer: None, .. } => "proposing a trade",
            Phase::Trading { .. } => "answering an offer",
            _ => "other",
        };
        self.decisions.push(Decision {
            round: game.round(),
            situation,
            base: describe(game, &base),
            chosen: describe(game, &chosen),
        });
        chosen
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
        for (n, wide) in [(3, false), (4, true), (5, false), (5, true)] {
            let mut game = Game::new(config.clone(), n, 11 + n as u64).unwrap();
            let mut agents: Vec<Box<dyn Agent>> = (0..n)
                .map(|i| -> Box<dyn Agent> {
                    if i == 0 {
                        let cfg = Config { budget: 8, wide, ..Config::default() };
                        Box::new(SearchAgent::with_config(1, cfg))
                    } else {
                        make_bot("house", i as u64)
                    }
                })
                .collect();
            play_out(&mut game, &mut agents);
            assert!(game.is_over());
        }
    }

    #[test]
    fn thread_count_does_not_change_play() {
        let config = Arc::new(GameConfig::embedded());
        let run = |threads: usize| {
            let mut game = Game::new(config.clone(), 4, 21).unwrap();
            let mut agents: Vec<Box<dyn Agent>> = (0..4)
                .map(|i| -> Box<dyn Agent> {
                    if i == 0 {
                        let cfg = Config { budget: 12, threads, ..Config::default() };
                        Box::new(SearchAgent::with_config(5, cfg))
                    } else {
                        make_bot("house", i as u64)
                    }
                })
                .collect();
            play_out(&mut game, &mut agents);
            game.history().to_vec()
        };
        assert_eq!(run(1), run(4));
    }

    #[test]
    fn determinized_copies_keep_public_state() {
        let config = Arc::new(GameConfig::embedded());
        let game = Game::new(config, 4, 3).unwrap();
        let d = game.determinized(99);
        assert_eq!(d.players(), game.players());
        assert_eq!(d.markets(), game.markets());
        let (mut a, mut b) = (d.bag().to_vec(), game.bag().to_vec());
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }
}
