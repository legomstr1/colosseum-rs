# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

A Rust engine for the 2007 board game Colosseum (Days of Wonder base game, 3–5 players), built mainly so AI agents can play it. There are front ends on top of one engine: a CLI and a browser GUI (both working), with an MCP/API layer and online multiplayer planned.

## Commands

```bash
cargo build --workspace
cargo test --workspace
cargo test -p colosseum-engine <name>              # run tests whose names match
cargo clippy --workspace --all-targets
cargo run -p colosseum-cli -- play --players human,greedy,greedy [--seed N]
cargo run -p colosseum-cli -- sim --players greedy,random,random --games 200
cargo run -p colosseum-cli -- --data data play     # load YAML from disk instead of the embedded copy
cargo run -p colosseum-web                         # browser GUI at http://127.0.0.1:8080
```

The workspace uses Rust edition 2024 and depends on `serde` and `serde_yml`, plus `clap` in the CLI and web server, and `axum`/`tokio` in the web server.

## Design principles (from the user)

- **No game data in code.** Every number lives in YAML (`data/game_config.yml`, `data/events.yml`): costs, supplies, bonuses, board layout, dice faces, auction minimums and so on. Rust code reads them from `GameConfig`. Asset and noble kinds are the only enums.
- **The game is driven by data structures.**
  - `Game` is plain, serializable data, including its RNG. Its fields are private: outside the `game` module it is read through getters (`players()`, `phase()`, `markets()`...) and changed only by `apply`. Tests that set up exact situations live in `game/tests.rs` for that reason.
  - Each `Phase` enum variant carries the state that only matters in that phase.
  - Every decision is an `Action` value.
  - Play advances only through `Game::apply(action) -> Result<Vec<GameEvent>, IllegalAction>`.
  - Keep new rules inside this reducer model. Don't add callbacks, trait-object phases or I/O in the engine.

## Architecture

- **`crates/colosseum-engine`**: the library.
  - `config.rs` deserializes the YAML. Its `validate()` checks consistency between files. The YAML is embedded with `include_str!("../../../data/...")`, and `GameConfig::from_dir` loads it at runtime.
  - `game/mod.rs` holds `Game`, setup, and the queries `current_player()`, `legal_actions()`, `check()` and `event_preview()`. It also holds the reducer: `apply` runs the phase handler, then `advance()` runs automatic steps (phase changes, dice rolls, refills, round end) until the next decision. `next_step()` decides what `advance` does next.
  - `game/{investing,acquiring,trading,producing,closing}.rs` each define one phase as a pair: `*_actions` (what is legal) and `apply_*` (the effect). Phase-specific state structs (`Acquiring`/`Auction`, `Offer`, `Producing`, `Closing`) live in the module for their phase.
  - `agent.rs` contains the `Agent` trait, `RandomAgent`, a heuristic `GreedyAgent` and `play_out`.
- **`crates/colosseum-cli`**: binary `colosseum`.
  - The `play` command mixes humans and bots at one terminal. Its menus collapse numeric action families (bids, medal counts) into a single prompt.
  - The `sim` command runs bot-only games and reports win rates.
- **`crates/colosseum-web`**: binary `colosseum-web`.
  - An axum server that owns every game in memory (`Session`: the `Game`, a seat kind per player, bot agents, a log of event strings). Bots move server-side until a human seat must act.
  - JSON API: `GET /api/config`, `POST /api/games` (`{seats: ["human","greedy","random"], seed?}`), `GET /api/games/{id}`, `POST /api/games/{id}/actions` (`{action}` in the engine's serde form, e.g. `{"Bid":{"amount":9}}` or `"PassBid"`). Every response is a `View`: public state plus the legal actions with `describe_action` text.
  - The page is vanilla HTML/CSS/JS in `static/`, embedded with `include_str!`, so changing it needs a rebuild and server restart. `app.js` redraws everything from each view. Token icons (silhouette SVG paths from game-icons.net, CC BY 3.0, in `static/icons.js`) and colors are presentation, not rules. Don't use emoji for icons. `Assets` serializes as a map like `{"Gladiator": 2}`.
  - Layout ("Stage Bench", chosen from the mockups in `crates/colosseum-web/mockups/`): light tan theme only. A top bar holds the round/phase tracker, Programs (`P`) and Log (`L`). The ring board has square spaces and picks whichever w×h border gives the biggest squares. Its middle is the stage: one function per phase (`investingStage`, `acquiringStage`, `tradingStage`, `producingStage`, `closingStage`) draws that phase's legal actions. Any legal action a stage doesn't draw still gets a plain button. Players sit on a bench underneath: the human's seat is wide, opponents are compact and show their programs on hover.
  - The board takes clicks: noble moves (pick a die, click a noble, click a faded noble on a dashed square) and arena expansion (the `+` squares). Dice show Roman numerals. Trading is a table between two hands: click tokens across, set coins, Offer / Counter.
  - Below 900px wide the stage moves under the ring and the bench stacks.
  - Every human seat plays from the same browser for now; seats in `Session` are where remote players would attach.
- **The legal-action contract**: `legal_actions()` lists every legal move except open-ended trade offers (`ProposeTrade`, `CounterTrade`), which `check()`/`is_legal()` validate. Actions available at any time (exchanging a medal for coins, or a SpecialEvent token for a medal) are added for whichever player is to act.
- **Determinism**: `Game` records `history`, and `Game::replay(config, n, seed, &history)` rebuilds the state exactly. Tests check this and the serde round-trip.
- **Tests**: `crates/colosseum-engine/tests/engine.rs` plays 120 full bot games and checks invariants after every action: standard tokens are conserved, the medal supply is never exceeded, and podiums are conserved.

## Rules sources and open questions

The rules were paraphrased from the RulesPal copy of the rulebook, then corrected by the user. Confirmed rulings:
- Dice faces are 2, 3, 4, 5, 6 and a "your choice of 1–3" face (`dice.faces` in YAML; a list is a choice face, and the chosen distance is part of the `MoveNoble`/`MoveNobleCombined` action).
- Any program for sale (11–30) can be bought; starting programs (1–10) never can.
- Jokers never count toward star awards or star bonuses.
- The closing donation (last place takes a token from the podium winner) can't take Jokers, Emperor or SpecialEvent tokens.
- Stars from the initial deal are awarded after every player is dealt; players tied for the most of a type all go without.
- The start player moves one seat left each round (the second player becomes the first).
- Players must use all dice before producing.
- An auction initiator who is outbid opens again.
- Markets refill when the initiator wins or passes, and at the end of the acquiring phase.
- The bank's coins are unlimited.

Not in the rulebook: trading caps (`max_offers_per_partner`, `max_counter_offers_per_partner`, 10 each per other player per turn) exist only to keep bot negotiations finite while leaving room for real haggling.

## Housekeeping

The old single-crate `src/` directory (`main.rs`, `types.rs`, `game.rs`) is obsolete. The workspace doesn't build it, and it can be deleted.
