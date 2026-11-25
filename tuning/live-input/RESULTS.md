# Live-input bot: sweep results (2026-11-21)

Five-hour training window. Every number below is the share of games won by
one live-input seat (unless noted), on the same seeds for every variant in a
run. With n players, an average bot wins 1/n of games (25% at 4 players).

## Summary

| Opponents | house | live-input (tuned) | live-input + search |
|---|---|---|---|
| 3 house bots, 4p (fresh seeds) | 25% | **37%** | 69% |
| house/oracle/maestro/chameleon, 3–5p | 35% | **48–52%** | 26–31% |
| holdout: trader/planner/showman/auctioneer, 4p | 73% | **87%** | 77% |
| 3 old search bots, 4p | 8% | **27%** | – |

The `live-input` seat plays the tuned heuristic. Search stays available
(`SearchConfig.budget`) but is off. Its playouts assume every opponent plays
like the rollout bot. That is a huge edge against house bots (69%) and a
large handicap against anyone else, and people aren't house bots.

## What changed from the house strategy

1. **Tuned planning numbers** (coordinate descent, about 400k games against house):
   - `fill_per_round` 2.0 → 0.4. The biggest single win: house was far too optimistic about filling gaps in an event later, so it chased events it couldn't complete.
   - `side_weight` 0.4 → 1.0, `spare_weight` 0.3 → 0.5, `top_events` 6 → 8, `open_gain` 1.0 → 0.5, `offers_per_turn` 4 → 6, `min_offer_gain` 1.0 → 0.6.
   - Against house: 24% → 37%. The gain carried over to bot fields the tuner never saw (above).
2. **Money trading**, following the ruling that you sell tokens for coins when you need money rather than cash medals:
   - A hand's value includes its coins. A coin is worth 0.5 points when the plan is short of money from round 4 on (a medal is 3 points or 6 coins), 0.15 when short earlier, and about 0.08 when money is plentiful.
   - The bot offers and accepts token-for-coin deals, counters near-miss offers by asking for coins, and stops offering money to players who refuse twice without ever accepting.
   - Money offers have their own budget (2 per turn) so they don't crowd out swaps.
   - Effect: +1.2 win points against the mixed field, +1.35 on the holdout, neutral against house (which never takes money). Two money bots at one table each gain a little and raise both scores; in all-live-input games the table average rises from 82.0 to 82.4.
   - Mistakes on the way: valuing coins high whenever the plan was short (even in round 2) made the bot dump tokens for cash: −3 to −6 points against the mixed field. Letting refused money offers use up swap slots cost 2 points against house.
3. **Tried and rejected**:
   - Mid-game programs (plans like `TEPE`, `LEPE`, `TEEP`): all worse (15–20% vs 24%). Dropping the loge or ticket costs more than an extra program gains against bots.
   - Reserving money for the final round's auction: −0.5 points.
   - Weighing medals against spectators when moving nobles (instead of medals always first): worse at every weight tried.

## Search findings

- Against house bots, search over the tuned heuristic: 68.8% (n=320), vs 37.8% for the heuristic alone.
- Against house/oracle/maestro/chameleon at 4p: full search 25.8%, investing-only search 48.3%, heuristic alone 52.5% (n=120). At 3–5p: search 31% vs 48% (n=200).
- With a mixed opponent model in the playouts (house, oracle, maestro, chameleon) on the holdout field: 90.8% vs 87.5% for the heuristic alone (n=120). That's within noise, and it is several times slower.

**Next step if search is wanted:** build the opponent model from human game logs (how often people accept money deals, bid, swap), or adapt it during the game, before turning search back on.

## Files

- `descend.py`: coordinate-descent tuner (`descend.log` has every batch, `best.txt` the result).
- `batch1-3.txt`, `search1-4.txt`, `vs_search.txt`: raw sweep outputs.
- Harness: `cargo run --release -p colosseum-bot-live-input --example sweep -- --games 2000 --field "house" --variants "house;core"` (options are listed at the top of `crates/bots/live-input/examples/sweep.rs`).
