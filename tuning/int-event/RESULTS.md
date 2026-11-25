# Int-event bot: results (2026-11-21)

Plan: season ticket (round 1), expansion (2), a tier-3 event that needs a
3-square arena (3), expansion (4), a tier-4 event that needs the full
4-square arena (final round). It chases podiums and turns SpecialEvent tokens
into a second season ticket when money allows.

## Does it follow the plan?

From `tests/plan.rs`: in 60 games against house bots it got a season ticket
in 60, a full arena in 60, and produced a tier-3 event in 57 and a tier-4
event in 51. It took 2.6 of the 4 podiums per game, and 16 games had two
tickets.

## Tuning

Coordinate descent against 3 house bots, 4 players (`descend.log`,
`best.txt`) raised its win rate from 13% to 23%. Accepted changes:
- `mid_cost_weight` 0.3 → 0.8: prefer cheaper tier-3 events.
- `fill_per_round` 0.4 → 0.2.
- `side_weight` 1.0 → 0.8, `spare_weight` 0.5 → 0.385.
- `commit_round` 2 → 1.
- `mid_weight` 1.0 → 0.7.

It then took tuned-1's trade search (two-token asks), giving the numbers below.

## Win rates (`validate.txt`, 1500 games per field, same seeds)

| Opponents | house | live-input | tuned-1 | int-event |
|---|---|---|---|---|
| 3 house bots, 4p | 26% | 39% | 47% | 32% |
| house/oracle/maestro/chameleon, 3–5p | 34% | 48% | 56% | 35% |
| holdout: trader/planner/showman/auctioneer, 4p | 70% | 84% | 89% | 75% |

## Where its points come from (`examples/diag.rs`, best event, vs house)

|  | total | event base | prev. programs | tickets | podiums | medals | nobles | coins at final investing |
|---|---|---|---|---|---|---|---|---|
| int-event | 82.3 | 41.4 | 14.0 | 5.9 | 8.1 | 6.1 | 5.6 | 68.5 |
| house | 79.7 | 45.0 | 9.7 | 5.4 | 1.3 | 12.1 | 7.2 | 43.8 |

The plan gains about 11 points from podiums and previous programs, but loses
about 6 in medals and 2 in nobles because it has no Emperor's loge (one die
instead of two), and its final event is worth about 4 less. It also reaches
the final round with about 25 more coins than house and never spends them.
More aggressive bids and SpecialEvent tickets didn't help in sweeps.
