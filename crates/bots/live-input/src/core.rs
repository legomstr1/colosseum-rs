//! The live-input heuristic: the house strategy with every number in
//! `Params`, plus what the analysis of human games showed it lacks:
//!
//! - **Money is a trade good.** A hand is valued as its tokens plus its coins,
//!   and a coin's value depends on whether the plan is short of money. When
//!   it is, selling a spare token for coins beats cashing a medal (6 coins
//!   cost 3 points); when it is not, coins can buy the tokens we need.
//!   Players who never take money deals stop being offered them.
//! - **Investing plan as data.** The plan is a string (`LTEE` is the house's
//!   loge, ticket, expansion, expansion) that may include `P`: buy a program
//!   that beats what we can produce. Sweeps found every `P` plan worse than
//!   `LTEE` against bots, so it stays `LTEE`.
//! - Planning numbers re-tuned by sweeps; the biggest change is expecting
//!   far fewer gaps to be filled later (`fill_per_round` 2.0 -> 0.4).

use crate::params::Params;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Action, Asset, Assets, EventDef, Game, Phase, PlayerId, Side, TradeTerms};
use std::collections::HashSet;

pub(crate) fn last_round(game: &Game) -> bool {
    game.round() >= game.rules().num_rounds
}

fn top_events(game: &Game, n: usize) -> Vec<u8> {
    let mut ids: Vec<&EventDef> = game
        .rules()
        .programs
        .for_sale
        .iter()
        .filter_map(|&id| game.config().event(id))
        .collect();
    ids.sort_by(|a, b| b.base_score.cmp(&a.base_score));
    ids.into_iter().take(n).map(|e| e.id).collect()
}

fn owned_top(game: &Game, who: PlayerId, tops: &[u8]) -> Option<u8> {
    game.player(who).programs.iter().map(|q| q.id).find(|id| tops.contains(id))
}

fn missing(def: &EventDef, assets: &Assets) -> Assets {
    let mut out = Assets::new();
    for (a, n) in def.requirement_counts().iter() {
        out.add(a, n.saturating_sub(assets.count(a)));
    }
    out
}

fn points_with_missing(def: &EventDef, missing: f64) -> f64 {
    let at = |m: usize| -> f64 {
        if let Some(s) = def.score_with_missing(m) {
            return s as f64;
        }
        let floor = def.penalty_scores.last().copied().unwrap_or(0) as f64;
        let over = m as f64 - def.max_missing() as f64;
        (floor - 4.0 * over).max(0.0) + 0.5 * (def.requirements.len() as f64 - m as f64).max(0.0)
    };
    let lo = missing.floor().max(0.0);
    let frac = missing - lo;
    at(lo as usize) * (1.0 - frac) + at(lo as usize + 1) * frac
}

pub(crate) fn event_points(def: &EventDef, assets: &Assets) -> f64 {
    let m = missing(def, assets).total().saturating_sub(assets.count(Asset::Joker));
    points_with_missing(def, m as f64)
}

fn rival_pick(game: &Game, q: PlayerId, tops: &[u8]) -> Option<(u8, f64)> {
    let assets = &game.player(q).assets;
    if let Some(id) = owned_top(game, q, tops) {
        return game.config().event(id).map(|d| (id, event_points(d, assets)));
    }
    tops.iter()
        .filter(|id| game.programs_for_sale().contains(id))
        .filter_map(|&id| game.config().event(id))
        .map(|d| (d.id, event_points(d, assets)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// What one player knows about the table, computed once per decision.
pub(crate) struct Ctx<'p> {
    p: &'p Params,
    me: PlayerId,
    tops: Vec<u8>,
    w: [f64; Asset::COUNT],
    pool: [f64; Asset::COUNT],
    rival_need: [f64; Asset::COUNT],
    rounds_left: f64,
    committed: bool,
    owned: Option<u8>,
    for_sale: Vec<(u8, f64)>,
}

impl<'p> Ctx<'p> {
    pub(crate) fn new(p: &'p Params, game: &Game, me: PlayerId) -> Ctx<'p> {
        let tops = top_events(game, p.top_events);
        let n = game.num_players();
        let mut played = game.discarded().clone();
        for q in game.players() {
            played.add_all(&q.assets);
        }
        let mut pool = [0f64; Asset::COUNT];
        for a in Asset::ALL {
            let total = game.rules().assets.total.get(&a).copied().unwrap_or(0);
            pool[a.index()] = total.saturating_sub(played.count(a)) as f64;
        }
        let mut demand = [0f64; Asset::COUNT];
        for &id in &tops {
            let produced = game.players().iter().any(|q| q.programs.iter().any(|r| r.id == id && r.produced));
            if produced {
                continue;
            }
            if let Some(def) = game.config().event(id) {
                for (a, k) in def.requirement_counts().iter() {
                    demand[a.index()] += k as f64;
                }
            }
        }
        let mut w = [0f64; Asset::COUNT];
        let standard: Vec<Asset> = Asset::ALL.into_iter().filter(|a| a.is_standard()).collect();
        for &a in &standard {
            w[a.index()] = (1.0 + demand[a.index()]) / (1.0 + pool[a.index()]);
        }
        let mean = standard.iter().map(|a| w[a.index()]).sum::<f64>() / standard.len() as f64;
        for a in Asset::ALL {
            w[a.index()] = if a.is_standard() {
                (w[a.index()] / mean).min(4.0)
            } else if a == Asset::Joker {
                2.5
            } else {
                0.5
            };
        }
        let rounds_to_go = game.rules().num_rounds.saturating_sub(game.round()) as usize;
        let start = (game.start_player().0 + rounds_to_go) % n;
        let order = |q: PlayerId| (q.0 + n - start) % n;
        let mine = &game.player(me).assets;
        let mut rival_need = [0f64; Asset::COUNT];
        let mut available: Vec<(u8, f64)> = tops.iter().map(|&id| (id, 1.0)).collect();
        for q in game.players().iter().filter(|q| q.id != me) {
            let Some((id, fit)) = rival_pick(game, q.id, &tops) else { continue };
            let Some(def) = game.config().event(id) else { continue };
            for (a, k) in missing(def, &q.assets).iter() {
                rival_need[a.index()] += k as f64;
            }
            let bought = owned_top(game, q.id, &tops).is_some();
            if !bought
                && order(q.id) < order(me)
                && fit >= event_points(def, mine)
                && let Some(e) = available.iter_mut().find(|e| e.0 == id)
            {
                e.1 *= p.rival_take;
            }
        }
        let before_producing =
            matches!(game.phase(), Phase::Investing { .. } | Phase::Acquiring(_) | Phase::Trading { .. });
        let rounds_left = rounds_to_go as f64 + if before_producing { 1.0 } else { 0.0 };
        let owned = owned_top(game, me, &tops);
        let for_sale = available.iter().copied().filter(|(id, _)| game.programs_for_sale().contains(id)).collect();
        Ctx { p, me, tops, w, pool, rival_need, rounds_left, committed: game.round() >= p.commit_round, owned, for_sale }
    }

    fn expected_points(&self, def: &EventDef, assets: &Assets) -> f64 {
        let gaps = missing(def, assets);
        let raw = gaps.total() as f64;
        let after_jokers = (raw - assets.count(Asset::Joker) as f64).max(0.0);
        if after_jokers == 0.0 {
            return points_with_missing(def, 0.0);
        }
        let mut fill = 0.0;
        for (a, k) in gaps.iter() {
            let pool = self.pool[a.index()];
            fill += k as f64 * pool / (pool + self.rival_need[a.index()] + 1.0);
        }
        let fill = (fill * after_jokers / raw).min(self.p.fill_per_round * self.rounds_left);
        points_with_missing(def, after_jokers - fill)
    }

    fn target(&self, game: &Game, assets: &Assets) -> Option<u8> {
        if let Some(id) = self.owned {
            return Some(id);
        }
        self.for_sale
            .iter()
            .filter_map(|&(id, avail)| game.config().event(id).map(|d| (d, avail)))
            .map(|(d, avail)| (self.expected_points(d, assets) * avail, d.id))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }
}

/// How much a hand of tokens is worth to `ctx.me` (coins not included).
pub(crate) fn plan_value(ctx: &Ctx, game: &Game, assets: &Assets) -> f64 {
    let p = ctx.p;
    let mut want = Assets::new();
    let mut value = 0.0;
    let target_weight = if ctx.committed { 1.0 } else { p.uncommitted_weight };
    if let Some(def) = ctx.target(game, assets).and_then(|id| game.config().event(id)) {
        value += target_weight * event_points(def, assets);
        for (a, n) in def.requirement_counts().iter() {
            want.add(a, n);
        }
    }
    if !last_round(game) {
        let me = game.player(ctx.me);
        let best = me
            .programs
            .iter()
            .filter_map(|q| game.config().event(q.id))
            .filter(|def| fits(game, ctx.me, def))
            .map(|def| (event_points(def, assets), def))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((pts, def)) = best {
            value += p.side_weight * pts;
            for (a, n) in def.requirement_counts().iter() {
                if want.count(a) < n {
                    want.add(a, n - want.count(a));
                }
            }
        }
    }
    for (a, n) in assets.iter() {
        let spare = n.saturating_sub(want.count(a));
        value += p.spare_weight * spare as f64 * ctx.w[a.index()];
    }
    value
}

fn fits(game: &Game, me: PlayerId, def: &EventDef) -> bool {
    let sq = game.rules().arena.squares_required_by_event_size.get(def.size as usize);
    sq.is_some_and(|&s| s <= game.player(me).arena.len)
}

/// Coins to spare beyond what the rest of the plan needs.
pub(crate) fn slack(ctx: &Ctx, game: &Game) -> i64 {
    let rules = game.rules();
    let p = game.player(ctx.me);
    let mut spend = 0i64;
    let last = last_round(game);
    if !last {
        if !p.loge && game.supply().emperor_loges > 0 && ctx.p.plan.contains('L') {
            spend += rules.investing.emperor_loge_cost as i64;
        }
        if p.season_tickets == 0 && ctx.p.plan.contains('T') {
            spend += rules.investing.season_ticket_cost as i64;
        }
        spend += rules.arena.max_squares.saturating_sub(p.arena.len) as i64 * rules.arena.expansion_cost as i64;
    }
    if owned_top(game, ctx.me, &ctx.tops).is_none()
        && let Some(def) = ctx.target(game, &p.assets).and_then(|id| game.config().event(id))
    {
        spend += def.cost as i64;
    }
    if last {
        return p.coins as i64 - spend;
    }
    let income = p
        .programs
        .iter()
        .filter_map(|q| game.event_preview(ctx.me, q.id, 0))
        .map(|pv| pv.breakdown.total())
        .max()
        .unwrap_or(0)
        .max(p.last_event.as_ref().map_or(0, |e| e.spectators)) as i64;
    let round = game.round() as i64;
    let last_n = rules.num_rounds as i64;
    let produced_this_round = p.last_event.as_ref().is_some_and(|e| e.round == game.round());
    let incomes = (last_n - round) - produced_this_round as i64;
    let mut later_auctions = (last_n - 1 - round).max(0);
    if ctx.p.reserve_final_auction {
        later_auctions += 1;
    }
    p.coins as i64 + income * incomes - spend - later_auctions * rules.acquiring.minimum_opening_bid as i64
}

/// Final-event points one coin is worth to `ctx.me` right now.
pub(crate) fn coin_value(ctx: &Ctx, game: &Game) -> f64 {
    let p = ctx.p;
    if last_round(game) {
        match game.phase() {
            Phase::Investing { .. } => {}
            Phase::Acquiring(_) | Phase::Trading { .. } => return p.coin_final,
            _ => return 0.0,
        }
    }
    let s = slack(ctx, game) as f64;
    let short = if game.round() >= p.short_from_round { p.coin_short } else { p.coin_early_short };
    if s < 0.0 {
        short
    } else if s >= p.coin_margin || p.coin_margin <= 0.0 {
        p.coin_rich
    } else {
        let t = s / p.coin_margin;
        short * (1.0 - t) + p.coin_rich * t
    }
}

fn with(assets: &Assets, add: &[Asset], remove: &[Asset]) -> Option<Assets> {
    let mut out = assets.clone();
    for &a in remove {
        if !out.remove(a, 1) {
            return None;
        }
    }
    for &a in add {
        out.add(a, 1);
    }
    Some(out)
}

/// (round, turn, partner, tokens given, tokens asked, coins received).
type OfferKey = (u32, usize, usize, Vec<Asset>, Vec<Asset>, i32);
/// (score, partner, tokens given, tokens asked, coins received).
type Candidate = (f64, PlayerId, Vec<Asset>, Vec<Asset>, i32);

/// The heuristic player.
pub struct Core {
    pub(crate) p: Params,
    rng: Rng,
    tried: HashSet<OfferKey>,
    /// (round, turn, swap offers, money offers) this trading turn.
    offers_made: (u32, usize, usize, usize),
    se_ticket: bool,
    /// Per player: money offers refused and accepted.
    money_log: Vec<(u32, u32)>,
    /// The offer we last made, with our coins and tokens before it, so the
    /// next decision can tell whether it was accepted.
    pending: Option<(PlayerId, bool, u32, Assets)>,
    /// (round, player) whose offers we already countered.
    countered: HashSet<(u32, usize)>,
}

impl Core {
    pub fn new(seed: u64, p: Params) -> Self {
        Core {
            p,
            rng: Rng::new(seed),
            tried: HashSet::new(),
            offers_made: (0, 0, 0, 0),
            se_ticket: false,
            money_log: Vec::new(),
            pending: None,
            countered: HashSet::new(),
        }
    }

    pub fn params(&self) -> &Params {
        &self.p
    }

    pub(crate) fn forget_pending(&mut self) {
        self.pending = None;
    }

    // ---------------------------------------------------------------- invest

    fn invest(&mut self, game: &Game, me: PlayerId, legal: &[Action], remaining: u32) -> Action {
        let p = self.p.clone();
        let ctx = Ctx::new(&p, game, me);
        let pl = game.player(me);
        let have_top = owned_top(game, me, &ctx.tops).is_some();
        let ticket_cost = game.rules().investing.season_ticket_cost as i64;

        if self.se_ticket && remaining > 0 && legal.contains(&Action::BuySeasonTicket) {
            self.se_ticket = false;
            return Action::BuySeasonTicket;
        }
        self.se_ticket = false;
        if remaining == 0 {
            let rich = slack(&ctx, game) >= ticket_cost && (have_top || !last_round(game));
            if rich && legal.contains(&Action::PlaySpecialEvent) && game.supply().season_tickets > 0 {
                self.se_ticket = true;
                return Action::PlaySpecialEvent;
            }
            return Action::EndInvesting;
        }

        if last_round(game) {
            if !have_top && let Some(a) = self.buy_final_event(&ctx, game, me, legal) {
                return a;
            }
            if legal.contains(&Action::BuySeasonTicket) && slack(&ctx, game) >= ticket_cost {
                return Action::BuySeasonTicket;
            }
            return Action::EndInvesting;
        }

        // A program clearly better than anything we can produce, out of order.
        if p.mid_buy_gain > 0.0
            && game.round() <= p.mid_buy_last_round
            && let Some((gain, a)) = self.mid_program(&ctx, game, me, legal)
            && gain >= p.mid_buy_gain
        {
            return a;
        }

        // Walk the plan: the first step not yet done.
        let max_sq = game.rules().arena.max_squares;
        let start_len = game.rules().starting_positions.get(&game.num_players()).and_then(|v| v.first()).map_or(2, |v| v.len() as u8);
        let mut tickets = pl.season_tickets;
        let mut expansions = pl.arena.len.saturating_sub(start_len);
        let mut programs = pl.programs.iter().filter(|q| game.rules().programs.for_sale.contains(&q.id)).count();
        for step in p.plan.chars() {
            match step {
                'L' => {
                    if !pl.loge && legal.contains(&Action::BuildLoge) {
                        return Action::BuildLoge;
                    }
                }
                'T' => {
                    if tickets > 0 {
                        tickets -= 1;
                    } else if legal.contains(&Action::BuySeasonTicket) {
                        return Action::BuySeasonTicket;
                    }
                }
                'E' => {
                    if expansions > 0 {
                        expansions -= 1;
                    } else if pl.arena.len < max_sq
                        && let Some(a) = expansion(game, me, legal)
                    {
                        return a;
                    }
                }
                'P' => {
                    if programs > 0 {
                        programs -= 1;
                    } else if let Some((gain, a)) = self.mid_program(&ctx, game, me, legal)
                        && gain > 0.0
                    {
                        return a;
                    }
                }
                _ => {}
            }
        }
        if legal.contains(&Action::BuySeasonTicket) && slack(&ctx, game) >= ticket_cost {
            return Action::BuySeasonTicket;
        }
        Action::EndInvesting
    }

    /// The affordable program that most beats our best producible event,
    /// with how many points it adds, if the money allows.
    fn mid_program(&self, ctx: &Ctx, game: &Game, me: PlayerId, legal: &[Action]) -> Option<(f64, Action)> {
        let pl = game.player(me);
        let best_now = pl
            .programs
            .iter()
            .filter_map(|q| game.config().event(q.id))
            .filter(|d| fits(game, me, d))
            .map(|d| event_points(d, &pl.assets))
            .fold(0.0, f64::max);
        let s = slack(ctx, game);
        legal
            .iter()
            .filter_map(|a| match a {
                Action::BuyProgram { program } => game.config().event(*program).map(|d| (d, a)),
                _ => None,
            })
            .filter(|(d, _)| fits(game, me, d))
            // A top event doubles as the final one: its cost is in the plan already.
            .filter(|(d, _)| ctx.tops.contains(&d.id) || (s - d.cost as i64) as f64 >= -self.p.mid_buy_slack)
            .map(|(d, a)| (ctx.expected_points(d, &pl.assets).min(event_points(d, &pl.assets) + 4.0) - best_now, a.clone()))
            .max_by(|a, b| a.0.total_cmp(&b.0))
    }

    fn buy_final_event(&mut self, ctx: &Ctx, game: &Game, me: PlayerId, legal: &[Action]) -> Option<Action> {
        let p = game.player(me);
        let best_now = p
            .programs
            .iter()
            .filter_map(|q| game.config().event(q.id))
            .map(|d| event_points(d, &p.assets))
            .fold(0.0, f64::max);
        let mut options: Vec<(f64, &EventDef)> = ctx
            .tops
            .iter()
            .filter(|id| game.programs_for_sale().contains(id))
            .filter_map(|&id| game.config().event(id))
            .map(|d| (ctx.expected_points(d, &p.assets), d))
            .collect();
        options.sort_by(|a, b| b.0.total_cmp(&a.0));
        let medal_coins = game.rules().medals.coin_value;
        for (pts, def) in &options {
            if *pts <= best_now {
                break;
            }
            let buy = Action::BuyProgram { program: def.id };
            if legal.contains(&buy) {
                return Some(buy);
            }
            let short = def.cost.saturating_sub(p.coins);
            let medals_needed = short.div_ceil(medal_coins);
            let medal_points = (medals_needed * game.rules().scoring.medal_bonus) as f64;
            if short > 0
                && medals_needed <= p.medals
                && pts - medal_points > best_now
                && legal.contains(&Action::ExchangeMedalForCoins)
            {
                return Some(Action::ExchangeMedalForCoins);
            }
        }
        legal
            .iter()
            .filter_map(|a| match a {
                Action::BuyProgram { program } => game.config().event(*program).map(|d| (event_points(d, &p.assets), a)),
                _ => None,
            })
            .filter(|(pts, _)| *pts > best_now + 3.0)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, a)| a.clone())
    }

    // --------------------------------------------------------------- acquire

    fn acquire(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Acquiring(acq) = game.phase() else { unreachable!() };
        let p = self.p.clone();
        let ctx = Ctx::new(&p, game, me);
        let pl = game.player(me);
        let base = plan_value(&ctx, game, &pl.assets);
        let medal_points = game.rules().scoring.medal_bonus as f64;
        let medal_coins = game.rules().medals.coin_value;
        let gain = |m: usize| {
            let mut after = pl.assets.clone();
            let mut emperors = 0u32;
            for &a in &game.markets()[m] {
                // An Emperor token becomes a medal on the spot (if any are left).
                if a == Asset::Emperor {
                    emperors += 1;
                } else {
                    after.add(a, 1);
                }
            }
            plan_value(&ctx, game, &after) - base + emperors.min(game.supply().medals) as f64 * medal_points
        };
        let final_round = last_round(game);
        let min = game.rules().acquiring.minimum_opening_bid;
        let spare = slack(&ctx, game);

        match &acq.auction {
            None => {
                let best = (0..game.markets().len())
                    .filter(|&m| !game.markets()[m].is_empty())
                    .map(|m| (gain(m), m))
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                let Some((g, market)) = best else { return Action::PassInitiative };
                // Nobody left who could outbid the minimum: the market costs
                // just that, and in the final round coins are only good for
                // trading, so anything useful (trade goods included) is worth it.
                let uncontested = game
                    .players()
                    .iter()
                    .all(|q| q.id == me || acq.won[q.id.0] || q.coins + q.medals * medal_coins <= min);
                let threshold = if final_round && uncontested {
                    min as f64 * coin_value(&ctx, game)
                } else if final_round {
                    p.final_open_gain
                } else {
                    p.open_gain
                };
                let worth = g > 0.0 && g >= threshold;
                if worth && pl.coins < min {
                    let medals_needed = (min - pl.coins).div_ceil(medal_coins);
                    // In the final round a cashed medal costs its points.
                    let pays = !final_round || g - (medals_needed as f64) * medal_points >= threshold;
                    if medals_needed <= pl.medals && pays && legal.contains(&Action::ExchangeMedalForCoins) {
                        return Action::ExchangeMedalForCoins;
                    }
                    return Action::PassInitiative;
                }
                let affordable = final_round || spare >= 0 || game.round() + 1 < game.rules().num_rounds;
                let open = Action::StartAuction { market, bid: min };
                if worth && affordable && legal.contains(&open) { open } else { Action::PassInitiative }
            }
            Some(auction) => {
                let g = gain(auction.market);
                let jokers = game.markets()[auction.market].iter().filter(|a| **a == Asset::Joker).count() as u32;
                let next = auction.high_bid + 1;
                let max_pay = if final_round {
                    if g >= 1.0 { (min as f64 + p.final_bid_mult * g) as u32 } else { 0 }
                } else if jokers > 0 {
                    min + (spare.max(0) as u32).min(p.joker_premium * jokers)
                } else if g >= p.big_batch_gain {
                    min + p.big_batch_premium
                } else {
                    0
                };
                let bid = Action::Bid { amount: next };
                if next <= max_pay && legal.contains(&bid) { bid } else { Action::PassBid }
            }
        }
    }

    // ----------------------------------------------------------------- trade

    fn note_pending(&mut self, game: &Game, me: PlayerId) {
        let Some((to, money, coins, assets)) = self.pending.take() else { return };
        if !money {
            return;
        }
        if self.money_log.len() < game.num_players() {
            self.money_log.resize(game.num_players(), (0, 0));
        }
        let pl = game.player(me);
        let changed = pl.coins != coins || pl.assets != assets;
        let e = &mut self.money_log[to.0];
        if changed { e.1 += 1 } else { e.0 += 1 }
    }

    fn money_ok(&self, q: PlayerId) -> bool {
        match self.money_log.get(q.0) {
            Some(&(refused, accepted)) => accepted > 0 || refused < self.p.money_refusals,
            None => true,
        }
    }

    fn propose(&mut self, game: &Game, me: PlayerId, turn: usize) -> Action {
        self.note_pending(game, me);
        if (self.offers_made.0, self.offers_made.1) != (game.round(), turn) {
            self.offers_made = (game.round(), turn, 0, 0);
            self.tried.clear();
        }
        let swaps_left = self.offers_made.2 < self.p.offers_per_turn;
        let money_left = self.offers_made.3 < self.p.money_offers_per_turn;
        if !swaps_left && !money_left {
            return Action::EndTrading;
        }
        let p = self.p.clone();
        let ctx = Ctx::new(&p, game, me);
        let w = ctx.w;
        let pl = game.player(me);
        let mine = &pl.assets;
        let base = plan_value(&ctx, game, mine);
        let my_cv = coin_value(&ctx, game);

        // Token parts: (give, get) with give empty, one, or two of a type and
        // get empty or one token. My gain from the tokens alone doesn't
        // depend on the partner.
        let mut gives: Vec<Vec<Asset>> = vec![vec![]];
        for (a, n) in mine.iter().filter(|(a, _)| a.is_tradeable()) {
            gives.push(vec![a]);
            if n >= 2 {
                gives.push(vec![a, a]);
            }
        }
        let mut best: Option<Candidate> = None;
        for q in game.players().iter().filter(|q| q.id != me) {
            let their_ctx = Ctx::new(&p, game, q.id);
            let their_base = plan_value(&their_ctx, game, &q.assets);
            let their_cv = coin_value(&their_ctx, game);
            let money = p.money_trades && money_left && self.money_ok(q.id);
            let mut gets: Vec<Vec<Asset>> = vec![vec![]];
            gets.extend(q.assets.iter().filter(|(a, _)| a.is_tradeable()).map(|(a, _)| vec![a]));
            for get in &gets {
                for give in &gives {
                    if give.is_empty() && get.is_empty() {
                        continue;
                    }
                    if give.len() == 2 && !get.is_empty() && w[give[0].index()] >= 0.6 * w[get[0].index()] {
                        continue;
                    }
                    if !give.is_empty() && !get.is_empty() && give[0] == get[0] {
                        continue;
                    }
                    if (give.is_empty() || get.is_empty()) && !money {
                        continue;
                    }
                    let Some(after) = with(mine, get, give) else { continue };
                    let my_tok = plan_value(&ctx, game, &after) - base;
                    let Some(theirs) = with(&q.assets, give, get) else { continue };
                    let their_tok = plan_value(&their_ctx, game, &theirs) - their_base;
                    // Coins I receive (negative: I pay). Pure swaps first,
                    // then the money ladder in both directions.
                    let mut ladder: Vec<i32> = Vec::new();
                    if swaps_left && !give.is_empty() && !get.is_empty() {
                        ladder.push(0);
                    }
                    if money {
                        for &c in &p.price_ladder {
                            if !give.is_empty() && c <= q.coins {
                                ladder.push(c as i32);
                            }
                            if !get.is_empty() && c <= pl.coins {
                                ladder.push(-(c as i32));
                            }
                        }
                    }
                    for coins in ladder {
                        if self.tried.contains(&(game.round(), turn, q.id.0, give.clone(), get.clone(), coins)) {
                            continue;
                        }
                        let my_gain = my_tok + my_cv * coins as f64;
                        if my_gain < p.min_offer_gain {
                            continue;
                        }
                        let their_gain = their_tok - their_cv * coins as f64;
                        if coins == 0 {
                            // House rule for swaps: fair by scarcity, or good for them.
                            let fair = give.iter().map(|a| w[a.index()]).sum::<f64>() - get.iter().map(|a| w[a.index()]).sum::<f64>();
                            if their_gain < -1.0 && fair < 0.0 {
                                continue;
                            }
                        } else if their_gain < p.partner_gain {
                            continue;
                        }
                        let score = my_gain + 0.5 * their_gain.min(2.0);
                        if best.as_ref().is_none_or(|b| score > b.0) {
                            best = Some((score, q.id, give.clone(), get.clone(), coins));
                        }
                    }
                }
            }
        }
        let Some((_, to, give, get, coins)) = best else { return Action::EndTrading };
        self.tried.insert((game.round(), turn, to.0, give.clone(), get.clone(), coins));
        if coins == 0 {
            self.offers_made.2 += 1;
        } else {
            self.offers_made.3 += 1;
        }
        let terms = TradeTerms {
            give: give.into_iter().collect(),
            give_coins: (-coins).max(0) as u32,
            receive: get.into_iter().collect(),
            receive_coins: coins.max(0) as u32,
        };
        let action = Action::ProposeTrade { to, terms };
        if game.is_legal(&action) {
            self.pending = Some((to, coins != 0, pl.coins, pl.assets.clone()));
            action
        } else {
            Action::EndTrading
        }
    }

    fn respond(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Trading { offer: Some(offer), .. } = game.phase() else {
            return Action::EndTrading;
        };
        if !legal.contains(&Action::AcceptTrade) {
            return Action::RejectTrade;
        }
        let t = &offer.terms;
        let p = self.p.clone();
        if !p.money_trades && (t.receive_coins > 0 || t.give_coins > 0) {
            return Action::RejectTrade;
        }
        let pl = game.player(me);
        if t.receive_coins > pl.coins {
            return Action::RejectTrade;
        }
        let mine = &pl.assets;
        let mut after = mine.clone();
        if !after.remove_all(&t.receive) {
            return Action::RejectTrade;
        }
        after.add_all(&t.give);
        let ctx = Ctx::new(&p, game, me);
        let cv = coin_value(&ctx, game);
        let coins = t.give_coins as f64 - t.receive_coins as f64;
        let gain = plan_value(&ctx, game, &after) - plan_value(&ctx, game, mine) + cv * coins;
        let w = ctx.w;
        let weigh = |s: &Assets| s.iter().map(|(a, n)| n as f64 * w[a.index()]).sum::<f64>();
        let fair = weigh(&t.give) - weigh(&t.receive);
        let swap = t.give_coins == 0 && t.receive_coins == 0;
        let need = if swap { p.accept_gain } else { p.accept_money_gain };
        if gain >= need || (swap && gain >= -0.2 && fair >= 0.0) {
            return Action::AcceptTrade;
        }
        // Close but not enough: ask the proposer to add coins, once per
        // partner per round, when coins matter to us and they have them.
        let short = need - gain;
        let key = (game.round(), offer.from.0);
        if p.counter_offers && p.money_trades && cv > 0.05 && self.money_ok(offer.from) && !self.countered.contains(&key) {
            let ask = (short / cv).ceil() as u32;
            let max = p.price_ladder.iter().copied().max().unwrap_or(0);
            let their = game.player(offer.from);
            // My view: I give what they asked for and get what they offered.
            let mut get_coins = t.give_coins as i64 - t.receive_coins as i64 + ask as i64;
            if ask > 0 && ask <= max && get_coins <= their.coins as i64 {
                let mut give_coins = 0i64;
                if get_coins < 0 {
                    give_coins = -get_coins;
                    get_coins = 0;
                }
                let terms = TradeTerms {
                    give: t.receive.clone(),
                    give_coins: give_coins as u32,
                    receive: t.give.clone(),
                    receive_coins: get_coins as u32,
                };
                let counter = Action::CounterTrade { terms };
                if game.is_legal(&counter) {
                    self.countered.insert(key);
                    self.pending = Some((offer.from, true, pl.coins, pl.assets.clone()));
                    return counter;
                }
            }
        }
        Action::RejectTrade
    }

    // --------------------------------------------------------------- produce

    fn produce(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let final_round = last_round(game);
        let dice_moves = legal
            .iter()
            .any(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }));
        if dice_moves {
            let (_, a) = best_dice_line(game, me, final_round, 2, self.p.dice_medal_weight);
            return a.expect("dice moves are legal");
        }
        if final_round {
            if legal.contains(&Action::ExchangeSpecialEventForMedal) {
                return Action::ExchangeSpecialEventForMedal;
            }
            let now = best_final_total(game, me);
            let mut best: Option<(u32, Action)> = None;
            for a in legal.iter().filter(|a| matches!(a, Action::MedalMoveNoble { .. })) {
                let mut next = game.clone();
                if next.apply(a.clone()).is_ok() {
                    let v = best_final_total(&next, me);
                    if v > now && best.as_ref().is_none_or(|b| v > b.0) {
                        best = Some((v, a.clone()));
                    }
                }
            }
            if let Some((_, a)) = best {
                return a;
            }
        }
        let pl = game.player(me);
        let medals = if final_round { pl.medals } else { 0 };
        let bonus = game.rules().scoring.previous_program_bonus as f64;
        legal
            .iter()
            .filter_map(|a| match a {
                Action::ProduceEvent { program, medals: m } if *m == medals => {
                    let pv = game.event_preview(me, *program, *m)?;
                    let fresh = pl.programs.iter().any(|q| q.id == *program && !q.produced);
                    let v = pv.breakdown.total() as f64 + if fresh && !final_round { bonus } else { 0.0 };
                    Some((v, a.clone()))
                }
                _ => None,
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map_or(Action::SkipEvent, |(_, a)| a)
    }

    // ----------------------------------------------------------------- close

    fn close(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let p = self.p.clone();
        let ctx = Ctx::new(&p, game, me);
        let mine = &game.player(me).assets;
        let scored = legal.iter().filter_map(|a| match a {
            Action::DiscardAsset { asset } => with(mine, &[], &[*asset]).map(|s| (plan_value(&ctx, game, &s), a)),
            Action::RequestAsset { asset } => with(mine, &[*asset], &[]).map(|s| (plan_value(&ctx, game, &s), a)),
            _ => None,
        });
        let mut best: Vec<(f64, &Action)> = Vec::new();
        for (v, a) in scored {
            match best.first() {
                Some((b, _)) if v < *b => {}
                Some((b, _)) if v == *b => best.push((v, a)),
                _ => best = vec![(v, a)],
            }
        }
        if best.is_empty() {
            return legal[0].clone();
        }
        best[self.rng.below(best.len())].1.clone()
    }

    pub fn choose(&mut self, game: &Game, legal: &[Action]) -> Action {
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
        if !matches!(game.phase(), Phase::Trading { offer: None, .. }) {
            self.note_pending(game, me);
        }
        let a = match game.phase() {
            Phase::Investing { remaining, .. } => self.invest(game, me, legal, *remaining),
            Phase::Acquiring(_) => self.acquire(game, me, legal),
            Phase::Trading { offer: None, turn, .. } => self.propose(game, me, *turn),
            Phase::Trading { .. } => self.respond(game, me, legal),
            Phase::Producing(_) => self.produce(game, me, legal),
            Phase::Closing(_) => self.close(game, me, legal),
            Phase::GameOver => legal[0].clone(),
        };
        if legal.contains(&a) || game.is_legal(&a) { a } else { legal[legal.len() - 1].clone() }
    }
}

fn best_final_total(game: &Game, me: PlayerId) -> u32 {
    let p = game.player(me);
    p.programs
        .iter()
        .filter_map(|q| game.event_preview(me, q.id, p.medals))
        .map(|pv| pv.breakdown.total())
        .max()
        .unwrap_or(0)
}

fn dice_value(game: &Game, me: PlayerId, final_round: bool, medal_w: f64) -> f64 {
    if final_round {
        return best_final_total(game, me) as f64;
    }
    let p = game.player(me);
    let best = p
        .programs
        .iter()
        .filter_map(|q| game.event_preview(me, q.id, 0))
        .map(|pv| pv.breakdown.total())
        .max()
        .unwrap_or(0) as f64;
    p.medals as f64 * medal_w + best
}

fn best_dice_line(game: &Game, me: PlayerId, final_round: bool, depth: u32, medal_w: f64) -> (f64, Option<Action>) {
    let still_rolling = matches!(game.phase(), Phase::Producing(pr) if pr.used.iter().any(|u| !u))
        && game.current_player() == Some(me);
    if depth == 0 || !still_rolling {
        return (dice_value(game, me, final_round, medal_w), None);
    }
    let mut best = (f64::NEG_INFINITY, None);
    for a in game.legal_actions() {
        if !matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }) {
            continue;
        }
        let mut next = game.clone();
        if next.apply(a.clone()).is_err() {
            continue;
        }
        let (v, _) = best_dice_line(&next, me, final_round, depth - 1, medal_w);
        if v > best.0 {
            best = (v, Some(a));
        }
    }
    best
}

fn expansion(game: &Game, me: PlayerId, legal: &[Action]) -> Option<Action> {
    let board = game.rules().board.size;
    let arena = &game.player(me).arena;
    let sides: Vec<Side> = [Side::Left, Side::Right]
        .into_iter()
        .filter(|s| legal.contains(&Action::ExpandArena { side: *s }))
        .collect();
    sides
        .iter()
        .max_by_key(|s| {
            let sq = arena.expansion_square(**s, board);
            let resting = game.rules().is_resting_place(sq) as u32;
            let noble_here = game.nobles().iter().any(|n| n.position == sq) as u32;
            resting * 2 + noble_here
        })
        .map(|s| Action::ExpandArena { side: *s })
}
