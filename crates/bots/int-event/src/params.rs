//! Every tunable number of the int-event bot, so sweeps can change them
//! without touching code. `Params::default()` is the tuned champion.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    // ----------------------------------------------------------- planning
    /// How many of the best for-sale events count as "the top events".
    pub top_events: usize,
    /// Round from which the final event counts fully in hand values.
    pub commit_round: u32,
    /// Weight of the final event in a hand's value before `commit_round`.
    pub uncommitted_weight: f64,
    /// Weight of the best program producible before the final round.
    pub side_weight: f64,
    /// Weight of each spare token (times its scarcity).
    pub spare_weight: f64,
    /// Useful tokens a round we expect to gain from auctions and trades.
    pub fill_per_round: f64,
    /// Chance a top event survives when a rival who buys first wants it.
    pub rival_take: f64,

    // ---------------------------------------------------------- investing
    /// One letter per round's investment before the final round:
    /// L = Emperor's loge, T = season ticket, E = expansion, P = the tier-3
    /// event (one that needs one square fewer than the largest arena).
    pub plan: String,
    /// Last round in which we still mean to buy a tier-3 event.
    pub mid_last_round: u32,
    /// Weight of the tier-3 event we mean to buy in a hand's value.
    pub mid_weight: f64,
    /// Points one coin of a tier-3 event's price costs when choosing it.
    pub mid_cost_weight: f64,
    /// Play a SpecialEvent token for a season ticket when spare money is at
    /// least this many tickets' price.
    pub se_ticket_slack: f64,
    /// Points added, before the final round, to an event that takes the
    /// lead in best single event (and so the round's podium).
    pub podium_weight: f64,
    /// Count the final round's opening bid when working out spare money.
    pub reserve_final_auction: bool,

    // ---------------------------------------------------------- acquiring
    pub open_gain: f64,
    pub final_open_gain: f64,
    pub joker_premium: u32,
    /// Final round: bid up to minimum + this times the batch's value.
    pub final_bid_mult: f64,
    /// Before the final round, bid up batches worth at least this much.
    pub big_batch_gain: f64,
    pub big_batch_premium: u32,

    // ------------------------------------------------------------ trading
    pub offers_per_turn: usize,
    /// Tokens an offer may ask for (1 or 2).
    pub max_get: usize,
    /// Weight of the partner's gain when ranking candidate offers, and the
    /// most of it that counts.
    pub their_gain_weight: f64,
    pub their_gain_cap: f64,
    /// Least gain (hand value, coins included) an offer must bring us.
    pub min_offer_gain: f64,
    /// Least gain we think the partner needs before accepting.
    pub partner_gain: f64,
    /// Accept offers worth at least this much to us.
    pub accept_gain: f64,
    /// Trade money as well as tokens.
    pub money_trades: bool,
    /// Coin amounts tried in money offers.
    pub price_ladder: Vec<u32>,
    /// Stop offering money deals to a player after this many refusals with
    /// no acceptance.
    pub money_refusals: u32,
    /// Money offers per trading turn, on top of `offers_per_turn` swaps.
    pub money_offers_per_turn: usize,
    /// Answer a near-miss offer by asking for more coins.
    pub counter_offers: bool,
    /// Value of a coin (in final-event points) when we are short of money
    /// for the plan: a medal is 3 points or 6 coins.
    pub coin_short: f64,
    /// `coin_short` applies from this round on (money is needed soon);
    /// before it, a short plan values coins at `coin_early_short`.
    pub short_from_round: u32,
    pub coin_early_short: f64,
    /// Accept offers involving money only when worth at least this much.
    pub accept_money_gain: f64,
    /// Value of a coin when money is plentiful.
    pub coin_rich: f64,
    /// Spare money over which a coin is worth `coin_rich`; between 0 and
    /// this, the value slides from `coin_short`.
    pub coin_margin: f64,
    /// Value of a coin in the final round after investing (bids only).
    pub coin_final: f64,

    // ---------------------------------------------------------- producing
    /// Dice: weight of a medal against one spectator now (100 = medals
    /// always first).
    pub dice_medal_weight: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            top_events: 8,
            commit_round: 1,
            uncommitted_weight: 0.5,
            side_weight: 0.8,
            spare_weight: 0.385,
            fill_per_round: 0.2,
            rival_take: 0.4,
            plan: "TEPE".into(),
            mid_last_round: 4,
            mid_weight: 0.7,
            mid_cost_weight: 0.8,
            se_ticket_slack: 1.0,
            podium_weight: 3.0,
            reserve_final_auction: false,
            open_gain: 0.5,
            final_open_gain: 3.0,
            joker_premium: 8,
            final_bid_mult: 3.0,
            big_batch_gain: 8.0,
            big_batch_premium: 2,
            offers_per_turn: 6,
            max_get: 2,
            their_gain_weight: 1.0,
            their_gain_cap: 2.0,
            min_offer_gain: 0.6,
            partner_gain: 0.0,
            accept_gain: 1.0,
            money_trades: true,
            price_ladder: vec![2, 4, 6, 9, 12, 15],
            money_refusals: 2,
            money_offers_per_turn: 2,
            counter_offers: true,
            coin_short: 0.5,
            short_from_round: 4,
            coin_early_short: 0.15,
            accept_money_gain: 2.0,
            coin_rich: 0.08,
            coin_margin: 12.0,
            coin_final: 0.03,
            dice_medal_weight: 100.0,
        }
    }
}

impl Params {
    /// Applies `key=value` overrides (numbers, bools, `plan=LTEE`,
    /// `price_ladder=2/4/6`). Unknown keys are an error.
    pub fn with_overrides(&self, spec: &str) -> Result<Params, String> {
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        for kv in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (k, val) = kv.split_once('=').ok_or_else(|| format!("bad override '{kv}'"))?;
            let obj = v.as_object_mut().unwrap();
            let old = obj.get(k).ok_or_else(|| format!("unknown param '{k}'"))?;
            let new = match old {
                serde_json::Value::Bool(_) => serde_json::Value::Bool(val.parse().map_err(|_| format!("bad bool {kv}"))?),
                serde_json::Value::String(_) => serde_json::Value::String(val.into()),
                serde_json::Value::Array(_) => serde_json::Value::Array(
                    val.split('/').map(|x| x.parse::<u64>().map(Into::into).map_err(|_| format!("bad list {kv}"))).collect::<Result<_, _>>()?,
                ),
                _ => serde_json::from_str(val).map_err(|_| format!("bad number {kv}"))?,
            };
            obj.insert(k.into(), new);
        }
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
}
