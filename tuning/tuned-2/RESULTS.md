# Tuned-2 bot: results (2026-11-21)

Tuned-2 is tuned-1 with two new params (`podium_weight`, `ticket_slack`) and a
wider tuning run against a stronger field: house/oracle/maestro/chameleon plus
tuned-1 and live-input, at 3–5 players. Trades stay mutually beneficial:
`partner_gain` (0) and `their_gain_weight` (1.0) were not tuned.

## Tuning (`descend.log`, 54 minutes, 3000 games a batch)

All 29 parameters were tried at least once. The changes that held up:
- `coin_early_short` 0.15 → 0.225: before round 4, coins are worth more when
  the plan is short of money.
- `accept_money_gain` 2.0 → 2.5: be choosier about offers involving money.
- `coin_rich` 0.128 → 0.192: coins are worth more even when money is plentiful.

The tuner also raised `offers_per_turn` to 13 and `money_offers_per_turn` to
4. Validation showed those make no difference, so tuned-2 keeps tuned-1's 10
and 3 because fewer offers are easier on people.

What didn't help: podium chasing (`podium_weight`, which int-event uses),
the extra-ticket threshold, other investing plans (`LTEP`, a program in the
fourth round, collapses to 3% wins), mid-game program buys, and the dice,
bidding and acceptance thresholds.

## Validation (`validate.txt`, fresh seeds)

| Opponents | tuned-1 | tuned-2 |
|---|---|---|
| house/oracle/maestro/chameleon/tuned-1/live-input, 3–5p (2500 games) | 47.9% | 50.1% |
| 3 house bots, 4p (2000) | 48.2% | 48.6% |
| holdout: trader/planner/showman/auctioneer, 4p (1500) | 90.9% | 91.9% |
| table average, 4 bots of one kind (600) | 82.60 | 83.11 |

The gain is small: about 2 points against the strong field, where the
standard error is 1 point, and within noise elsewhere. Tables of tuned-2 bots
score more than tables of tuned-1 bots, so the trades still create points.

## No worse offer after a refusal (2026-11-22)

Feedback from a live game: after P1 refused "Gladiator + 4 coins for Comedian
and Decoration", the bot offered the same trade for 2 coins, then for 0.
Now each refused offer is remembered for the round. An offer is skipped if it
is no better for that partner: it gives them the same tokens or fewer, asks
for the same or more, and asks for at least as many coins. A refused counter
offer counts too. `tests/legal.rs` checks this in 24 games against house bots.
On the same seeds as above it wins 50.3% (was 50.1%) on the strong field, and
the table average is 83.07 (was 83.11), so the change costs nothing.
