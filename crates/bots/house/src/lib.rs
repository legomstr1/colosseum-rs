//! House: the strategy the user's group plays in person (100+ games, 4-5
//! players, final events usually scoring 80-100).
//!
//! - Investing, one per round: Emperor's loge, season ticket, expansion,
//!   expansion, then one of the top 6 events (by base score) in the final
//!   round.
//! - From round 2 on, aim at the top-6 event we expect to field best: our
//!   tokens, the tokens still in the bag, what rivals chase, and whether a
//!   rival who invests before us in the final round will likely take it.
//! - Acquiring: open auctions at the minimum bid; almost never bid up before
//!   the final round, except for Jokers, as far as spare money allows
//!   (counting the income still to come and the investments still planned).
//!   In the final round coins have no other use, so bid.
//! - Trading: token-for-token only, weighting each asset type by how scarce
//!   it is (total tokens of the type, how many of the top-6 events need it,
//!   how many are already out of the bag). Money is never traded.
//! - SpecialEvent tokens buy an extra season ticket when money is plentiful,
//!   otherwise they become medals.
//! - Producing: dice go to earning medals. Medals are kept and only spent in
//!   the final round, as spectators or as noble moves when a move is worth
//!   more than the 3 points a medal adds; for coins only when the final
//!   event, or any auction at all, can't be paid for otherwise.

use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Action, Asset, Assets, EventDef, Game, Phase, PlayerId, Side, TradeTerms};
use std::collections::HashSet;

/// How many of the best for-sale events count as "the top events".
const TOP_EVENTS: usize = 6;
/// Round from which the bot commits to a final event.
const COMMIT_ROUND: u32 = 2;
/// Offers the bot makes per trading turn.
const OFFERS_PER_TURN: usize = 4;
/// Most coins over the minimum bid the bot pays per Joker in a batch.
const JOKER_PREMIUM: u32 = 8;

pub struct HouseAgent {
    rng: Rng,
    /// Trades already proposed this trading turn, keyed by (round, turn).
    tried: HashSet<(u32, usize, usize, Asset, Asset)>,
    offers_made: (u32, usize, usize),
    /// A SpecialEvent token was just played to buy an extra season ticket.
    se_ticket: bool,
}

impl HouseAgent {
    pub fn new(seed: u64) -> Self {
        Self { rng: Rng::new(seed), tried: HashSet::new(), offers_made: (0, 0, 0), se_ticket: false }
    }
}

fn last_round(game: &Game) -> bool {
    game.round() >= game.rules().num_rounds
}

/// The top events (highest base score) among programs that can be bought.
fn top_events(game: &Game) -> Vec<u8> {
    let mut ids: Vec<&EventDef> = game
        .rules()
        .programs
        .for_sale
        .iter()
        .filter_map(|&id| game.config().event(id))
        .collect();
    ids.sort_by(|a, b| b.base_score.cmp(&a.base_score));
    ids.into_iter().take(TOP_EVENTS).map(|e| e.id).collect()
}

/// The owned top event, if any.
fn owned_top(game: &Game, who: PlayerId, tops: &[u8]) -> Option<u8> {
    game.player(who).programs.iter().map(|q| q.id).find(|id| tops.contains(id))
}

/// Missing tokens per type for `def`, before Jokers.
fn missing(def: &EventDef, assets: &Assets) -> Assets {
    let mut out = Assets::new();
    for (a, n) in def.requirement_counts().iter() {
        out.add(a, n.saturating_sub(assets.count(a)));
    }
    out
}

/// Base score of `def` with `missing` tokens short (fractional counts
/// interpolate). Past the last penalty step the score keeps dropping, so
/// partial progress still counts.
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

/// Base score of `def` with `assets` (Jokers fill gaps).
fn event_points(def: &EventDef, assets: &Assets) -> f64 {
    let m = missing(def, assets).total().saturating_sub(assets.count(Asset::Joker));
    points_with_missing(def, m as f64)
}

/// A rival's likely final event: the top event they own, or else their best
/// match among those still for sale.
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
struct Ctx {
    me: PlayerId,
    tops: Vec<u8>,
    /// Scarcity weight per asset type, averaging about 1.
    w: [f64; Asset::COUNT],
    /// Tokens of each type still unplayed (not in anyone's hands or discarded).
    pool: [f64; Asset::COUNT],
    /// Tokens rivals still need for the top event they are likely after.
    rival_need: [f64; Asset::COUNT],
    /// Rounds of acquiring and trading still to come, counting this one.
    rounds_left: f64,
    committed: bool,
    /// The top event `me` owns, if any.
    owned: Option<u8>,
    /// Top events still for sale, with the chance each is still for sale
    /// when we buy in the final round.
    for_sale: Vec<(u8, f64)>,
}

impl Ctx {
    fn new(game: &Game, me: PlayerId) -> Ctx {
        let tops = top_events(game);
        let n = game.num_players();

        // Played tokens are out of the bag: in someone's hands or discarded.
        let mut played = game.discarded().clone();
        for p in game.players() {
            played.add_all(&p.assets);
        }
        let mut pool = [0f64; Asset::COUNT];
        for a in Asset::ALL {
            let total = game.rules().assets.total.get(&a).copied().unwrap_or(0);
            pool[a.index()] = total.saturating_sub(played.count(a)) as f64;
        }

        // Demand: copies the top events still in play require.
        let mut demand = [0f64; Asset::COUNT];
        for &id in &tops {
            let produced = game.players().iter().any(|p| p.programs.iter().any(|q| q.id == id && q.produced));
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

        // Rivals: what they need, and which events they will likely buy
        // before us. Final-round investing starts `rounds_to_go` seats on.
        let rounds_to_go = game.rules().num_rounds.saturating_sub(game.round()) as usize;
        let start = (game.start_player().0 + rounds_to_go) % n;
        let order = |p: PlayerId| (p.0 + n - start) % n;
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
                e.1 *= 0.4;
            }
        }

        let before_producing =
            matches!(game.phase(), Phase::Investing { .. } | Phase::Acquiring(_) | Phase::Trading { .. });
        let rounds_left = rounds_to_go as f64 + if before_producing { 1.0 } else { 0.0 };
        let owned = owned_top(game, me, &tops);
        let for_sale = available.iter().copied().filter(|(id, _)| game.programs_for_sale().contains(id)).collect();
        Ctx { me, tops, w, pool, rival_need, rounds_left, committed: game.round() >= COMMIT_ROUND, owned, for_sale }
    }

    /// Score we expect from `def` by the final round: current holdings plus
    /// the share of each gap we can hope to fill, given the tokens still in
    /// the bag and how many of them rivals are chasing too.
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
        // About two useful tokens a round from an auction and trades.
        let fill = (fill * after_jokers / raw).min(2.0 * self.rounds_left);
        points_with_missing(def, after_jokers - fill)
    }

    /// The top event to aim at: the one we own, or else the one we best
    /// expect to complete and still be able to buy.
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

/// How much a hand of tokens is worth to `ctx.me`: the final event first,
/// the programs it can produce before then second, spare tokens as trade
/// bait.
fn plan_value(ctx: &Ctx, game: &Game, assets: &Assets) -> f64 {
    let mut want = Assets::new();
    let mut value = 0.0;

    let target_weight = if ctx.committed { 1.0 } else { 0.5 };
    if let Some(def) = ctx.target(game, assets).and_then(|id| game.config().event(id)) {
        value += target_weight * event_points(def, assets);
        for (a, n) in def.requirement_counts().iter() {
            want.add(a, n);
        }
    }

    if !last_round(game) {
        let p = game.player(ctx.me);
        let best = p
            .programs
            .iter()
            .filter_map(|q| game.config().event(q.id))
            .filter(|def| {
                let sq = game.rules().arena.squares_required_by_event_size.get(def.size as usize);
                sq.is_some_and(|&s| s <= p.arena.len)
            })
            .map(|def| (event_points(def, assets), def))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((pts, def)) = best {
            value += 0.4 * pts;
            for (a, n) in def.requirement_counts().iter() {
                if want.count(a) < n {
                    want.add(a, n - want.count(a));
                }
            }
        }
    }

    for (a, n) in assets.iter() {
        let spare = n.saturating_sub(want.count(a));
        value += 0.3 * spare as f64 * ctx.w[a.index()];
    }
    value
}

/// Coins to spare beyond what the rest of the plan needs: current coins plus
/// the event income still to come before the final round, minus the
/// investments still planned and a minimum-bid auction each later round.
fn slack(ctx: &Ctx, game: &Game) -> i64 {
    let rules = game.rules();
    let p = game.player(ctx.me);
    let mut spend = 0i64;
    if !p.loge && game.supply().emperor_loges > 0 {
        spend += rules.investing.emperor_loge_cost as i64;
    }
    if p.season_tickets == 0 {
        spend += rules.investing.season_ticket_cost as i64;
    }
    spend += rules.arena.max_squares.saturating_sub(p.arena.len) as i64 * rules.arena.expansion_cost as i64;
    if owned_top(game, ctx.me, &ctx.tops).is_none()
        && let Some(def) = ctx.target(game, &p.assets).and_then(|id| game.config().event(id))
    {
        spend += def.cost as i64;
    }
    if last_round(game) {
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
    let last = rules.num_rounds as i64;
    let produced_this_round = p.last_event.as_ref().is_some_and(|e| e.round == game.round());
    // Event income still to come before the final round's investing.
    let incomes = (last - round) - produced_this_round as i64;
    let later_auctions = (last - 1 - round).max(0);
    p.coins as i64 + income * incomes - spend - later_auctions * rules.acquiring.minimum_opening_bid as i64
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

impl HouseAgent {
    // ---------------------------------------------------------------- invest

    fn invest(&mut self, game: &Game, me: PlayerId, legal: &[Action], remaining: u32) -> Action {
        let ctx = Ctx::new(game, me);
        let p = game.player(me);
        let have_top = owned_top(game, me, &ctx.tops).is_some();
        let ticket_cost = game.rules().investing.season_ticket_cost as i64;

        // A SpecialEvent token buys an extra season ticket when money is
        // plentiful; otherwise it is kept and turned into a medal.
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

        // The final slot: buy the top event we best match.
        if last_round(game)
            && !have_top
            && let Some(a) = self.buy_final_event(&ctx, game, me, legal)
        {
            return a;
        }

        let max_sq = game.rules().arena.max_squares;
        let plan: [&dyn Fn() -> Option<Action>; 3] = [
            &|| (!p.loge && legal.contains(&Action::BuildLoge)).then_some(Action::BuildLoge),
            &|| (p.season_tickets == 0 && legal.contains(&Action::BuySeasonTicket)).then_some(Action::BuySeasonTicket),
            &|| if p.arena.len < max_sq { expansion(game, me, legal) } else { None },
        ];
        if let Some(a) = plan.iter().find_map(|f| f()) {
            return a;
        }
        // Plan done early (or blocked): another season ticket, if it leaves
        // the money for the final event.
        if legal.contains(&Action::BuySeasonTicket) && slack(&ctx, game) >= ticket_cost {
            return Action::BuySeasonTicket;
        }
        Action::EndInvesting
    }

    fn buy_final_event(&mut self, ctx: &Ctx, game: &Game, me: PlayerId, legal: &[Action]) -> Option<Action> {
        let p = game.player(me);
        let best_now = p
            .programs
            .iter()
            .filter_map(|q| game.config().event(q.id))
            .map(|d| event_points(d, &p.assets))
            .fold(0.0, f64::max);
        // Each for-sale top event, by what we expect to field after this
        // round's auction and trades.
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
            // Medals for money only when we absolutely have to: the gap must
            // be closable and the event worth more than the medals' points.
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
        // No top event fits: the best affordable program that beats what we have.
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
        let ctx = Ctx::new(game, me);
        let p = game.player(me);
        let base = plan_value(&ctx, game, &p.assets);
        let gain = |m: usize| {
            let mut after = p.assets.clone();
            for &a in &game.markets()[m] {
                after.add(a, 1);
            }
            plan_value(&ctx, game, &after) - base
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
                let worth = g >= if final_round { 3.0 } else { 1.0 };
                // Short of the minimum bid: medals become coins only when that
                // is the only way to buy at all.
                if worth && p.coins < min {
                    let medals_needed = (min - p.coins).div_ceil(game.rules().medals.coin_value);
                    if medals_needed <= p.medals && legal.contains(&Action::ExchangeMedalForCoins) {
                        return Action::ExchangeMedalForCoins;
                    }
                    return Action::PassInitiative;
                }
                // Keep the money for the final event (after this round's income).
                let affordable = final_round || spare >= 0 || game.round() + 1 < game.rules().num_rounds;
                let open = Action::StartAuction { market, bid: min };
                if worth && affordable && legal.contains(&open) { open } else { Action::PassInitiative }
            }
            Some(auction) => {
                let g = gain(auction.market);
                let jokers = game.markets()[auction.market].iter().filter(|a| **a == Asset::Joker).count() as u32;
                let next = auction.high_bid + 1;
                let max_pay = if final_round {
                    // Coins are worth nothing after the final investment.
                    if g >= 1.0 { (min as f64 + 3.0 * g) as u32 } else { 0 }
                } else if jokers > 0 {
                    // Jokers are worth bidding up, as far as the money we can
                    // spare (this round's and later income included) allows.
                    min + (spare.max(0) as u32).min(JOKER_PREMIUM * jokers)
                } else if g >= 8.0 {
                    // Otherwise only an exceptional batch is worth bidding up.
                    min + 2
                } else {
                    0
                };
                let bid = Action::Bid { amount: next };
                if next <= max_pay && legal.contains(&bid) { bid } else { Action::PassBid }
            }
        }
    }

    // ----------------------------------------------------------------- trade

    fn propose(&mut self, game: &Game, me: PlayerId, turn: usize) -> Action {
        let key = (game.round(), turn, 0);
        if (self.offers_made.0, self.offers_made.1) != (key.0, key.1) {
            self.offers_made = key;
            self.tried.clear();
        }
        if self.offers_made.2 >= OFFERS_PER_TURN {
            return Action::EndTrading;
        }
        let ctx = Ctx::new(game, me);
        let w = ctx.w;
        let mine = &game.player(me).assets;
        let base = plan_value(&ctx, game, mine);
        let mut best: Option<(f64, PlayerId, Vec<Asset>, Asset)> = None;
        // My gain from a swap doesn't depend on the partner.
        let mut my_gains: Vec<((Asset, Asset), Option<f64>)> = Vec::new();
        for q in game.players().iter().filter(|q| q.id != me) {
            let their_ctx = Ctx::new(game, q.id);
            let their_base = plan_value(&their_ctx, game, &q.assets);
            for (get, _) in q.assets.iter().filter(|(a, _)| a.is_tradeable()) {
                for (give, _) in mine.iter().filter(|(a, _)| a.is_tradeable() && *a != get) {
                    if self.tried.contains(&(game.round(), turn, q.id.0, give, get)) {
                        continue;
                    }
                    // One-for-one when the weights roughly match, two-for-one
                    // when we hand over a commoner type.
                    let mut gives = vec![give];
                    if w[give.index()] < 0.6 * w[get.index()] && mine.count(give) >= 2 {
                        gives.push(give);
                    }
                    let my_gain = match my_gains.iter().find(|(k, _)| *k == (give, get)) {
                        Some((_, g)) => *g,
                        None => {
                            let g = with(mine, &[get], &gives).map(|after| plan_value(&ctx, game, &after) - base);
                            my_gains.push(((give, get), g));
                            g
                        }
                    };
                    let Some(my_gain) = my_gain else { continue };
                    if my_gain < 1.0 {
                        continue;
                    }
                    // Would they take it, by our own yardstick?
                    let Some(theirs) = with(&q.assets, &gives, &[get]) else { continue };
                    let their_gain = plan_value(&their_ctx, game, &theirs) - their_base;
                    let fair = gives.iter().map(|a| w[a.index()]).sum::<f64>() - w[get.index()];
                    if their_gain < -1.0 && fair < 0.0 {
                        continue;
                    }
                    let score = my_gain + 0.5 * their_gain.min(2.0);
                    if best.as_ref().is_none_or(|b| score > b.0) {
                        best = Some((score, q.id, gives, get));
                    }
                }
            }
        }
        let Some((_, to, gives, get)) = best else { return Action::EndTrading };
        self.tried.insert((game.round(), turn, to.0, gives[0], get));
        self.offers_made.2 += 1;
        let terms = TradeTerms {
            give: gives.into_iter().collect(),
            give_coins: 0,
            receive: [get].into_iter().collect(),
            receive_coins: 0,
        };
        let action = Action::ProposeTrade { to, terms };
        if game.is_legal(&action) { action } else { Action::EndTrading }
    }

    fn respond(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let Phase::Trading { offer: Some(offer), .. } = game.phase() else {
            return Action::EndTrading;
        };
        if !legal.contains(&Action::AcceptTrade) {
            return Action::RejectTrade;
        }
        let t = &offer.terms;
        // We never pay money for tokens.
        if t.receive_coins > 0 {
            return Action::RejectTrade;
        }
        let mine = &game.player(me).assets;
        let mut after = mine.clone();
        if !after.remove_all(&t.receive) {
            return Action::RejectTrade;
        }
        after.add_all(&t.give);
        let ctx = Ctx::new(game, me);
        let gain = plan_value(&ctx, game, &after) - plan_value(&ctx, game, mine);
        let w = ctx.w;
        let weigh = |s: &Assets| s.iter().map(|(a, n)| n as f64 * w[a.index()]).sum::<f64>();
        let fair = weigh(&t.give) - weigh(&t.receive);
        if gain >= 1.0 || (gain >= -0.2 && fair >= 0.0) {
            Action::AcceptTrade
        } else {
            Action::RejectTrade
        }
    }

    // --------------------------------------------------------------- produce

    fn produce(&mut self, game: &Game, me: PlayerId, legal: &[Action]) -> Action {
        let final_round = last_round(game);
        let dice_moves = legal
            .iter()
            .any(|a| matches!(a, Action::MoveNoble { .. } | Action::MoveNobleCombined { .. }));
        if dice_moves {
            let (_, a) = best_dice_line(game, me, final_round, 2);
            return a.expect("dice moves are legal");
        }

        if final_round {
            if legal.contains(&Action::ExchangeSpecialEventForMedal) {
                return Action::ExchangeSpecialEventForMedal;
            }
            // A medal move only if it beats keeping the medal as 3 points.
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

        let p = game.player(me);
        let medals = if final_round { p.medals } else { 0 };
        let bonus = game.rules().scoring.previous_program_bonus as f64;
        legal
            .iter()
            .filter_map(|a| match a {
                Action::ProduceEvent { program, medals: m } if *m == medals => {
                    let pv = game.event_preview(me, *program, *m)?;
                    let fresh = p.programs.iter().any(|q| q.id == *program && !q.produced);
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
        let ctx = Ctx::new(game, me);
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
}

/// Best event total right now, spending every medal.
fn best_final_total(game: &Game, me: PlayerId) -> u32 {
    let p = game.player(me);
    p.programs
        .iter()
        .filter_map(|q| game.event_preview(me, q.id, p.medals))
        .map(|pv| pv.breakdown.total())
        .max()
        .unwrap_or(0)
}

/// How good a position is after the dice: medals first in early rounds,
/// the final event's total in the last round.
fn dice_value(game: &Game, me: PlayerId, final_round: bool) -> f64 {
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
    p.medals as f64 * 100.0 + best
}

/// Searches the remaining dice moves (at most two dice) for the best line.
fn best_dice_line(game: &Game, me: PlayerId, final_round: bool, depth: u32) -> (f64, Option<Action>) {
    let still_rolling = matches!(game.phase(), Phase::Producing(pr) if pr.used.iter().any(|u| !u))
        && game.current_player() == Some(me);
    if depth == 0 || !still_rolling {
        return (dice_value(game, me, final_round), None);
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
        let (v, _) = best_dice_line(&next, me, final_round, depth - 1);
        if v > best.0 {
            best = (v, Some(a));
        }
    }
    best
}

/// Expansion side that keeps the other side open for the next one.
fn expansion(game: &Game, me: PlayerId, legal: &[Action]) -> Option<Action> {
    let board = game.rules().board.size;
    let arena = &game.player(me).arena;
    let sides: Vec<Side> = [Side::Left, Side::Right]
        .into_iter()
        .filter(|s| legal.contains(&Action::ExpandArena { side: *s }))
        .collect();
    // Prefer a side whose square holds or is next to a noble's resting place.
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

impl Agent for HouseAgent {
    fn name(&self) -> String {
        "house".into()
    }

    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let Some(me) = game.current_player() else {
            return legal[0].clone();
        };
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
