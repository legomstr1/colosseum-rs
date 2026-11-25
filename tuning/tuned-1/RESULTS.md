# Tuned-1 bot: results (2026-11-21)

Goal: trades a person might actually accept, ones that are mutually
beneficial and raise the table's scores, without costing win rate against
live-input.

## Rules

- An offer must leave the partner no worse off by the bot's model of their
  hand (`partner_gain` = 0, which the tuner was not allowed to change).
  Live-input also offered swaps that were "fair by token scarcity" or cost
  the partner up to a point; house bots accept those, people don't.
- It never asks for a Joker unless it gives more tokens than it asks for, or
  pays coins.
- Offers may ask for two tokens (`max_get` = 2). This is the big gain: there
  are many more trades that help both sides.
- Candidates are ranked by the bot's gain + 1.0 × the partner's gain (capped at 2).

## What cost points at first (`vs_*.txt`, 1500–2000 games)

The first version also capped offers at 2 per partner per turn. It won 2–3
points less than live-input. Most of that was the cap: without it the bot was
level with live-input against the mixed field, and 1.7 points behind against
house. The cap was removed.

## Tuning

Coordinate descent against house/oracle/maestro/chameleon, 4 players
(`descend.log`, `best.txt`) gave `offers_per_turn` 6 → 10,
`money_offers_per_turn` 2 → 3, `money_refusals` 2 → 3, `side_weight`
1.0 → 1.25 and `coin_rich` 0.08 → 0.128. It also found `their_gain_weight`
0.5, but 1.0 was kept because it wins within noise and partners score more.

## Validation (`validate.txt`, 2000 games per field, fresh seeds)

| Opponents | live-input | tuned-1 |
|---|---|---|
| 3 house bots, 4p | 37.0% | 46.4% |
| house/oracle/maestro/chameleon, 3–5p | 50.3% | 58.6% |
| holdout: trader/planner/showman/auctioneer, 4p | 85.0% | 91.2% |
| table average, 4 bots of one kind | 82.40 | 83.04 |

The last row is the mutual-benefit check: tables made up entirely of tuned-1
bots score higher than tables made up entirely of live-input bots, so the
extra trades create points instead of only moving them between players.
