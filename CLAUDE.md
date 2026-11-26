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
crates/colosseum-wasm/build.sh                     # serverless GUI (WebAssembly) into dist/; serve it statically
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
- **`crates/bots/*`**: one crate per bot. `house` is the in-person group's strategy; `search` is Monte Carlo search over `house`.
- **`crates/bots/live-input`** (seat `live-input`): the strongest bot, built from lessons of live games against people.
  - `core.rs`: the house plan with every number in `Params` (`params.rs`, tuned by sweeps), plus money trading. A hand's value includes its coins, and a coin is worth more when the plan is short of money in the rounds just before the final purchase. So the bot sells spare tokens for coins rather than cashing medals (6 coins cost 3 points), buys needed tokens when rich, counters near-miss offers by asking for coins, and stops offering money to players who keep refusing. Money offers have their own per-turn budget so they don't crowd out token swaps.
  - `search.rs`: `LiveInputAgent`, the seat's agent. It can run successive-halving Monte Carlo search over the heuristic (heuristic in its own seat, `rollout` bots in the others), but `budget` is 0 by default. Search wins 69% against house bots, yet does much worse than the heuristic alone against any other opponents, because its playouts assume the opponents play like the rollout bot.
  - `examples/sweep.rs`: the tuning harness. It plays variants on the same seeds against a field and reports win %, placement and score. Variant specs are `core{key=value,...}`, `li{...,s.budget=96}`, `t1{...}` (tuned-1), `t2{...}` (tuned-2) and `ie{...}` (int-event). `tuning/live-input/` holds the coordinate-descent tuner and its logs.
- **`crates/bots/tuned-1`** (seat `tuned-1`): a fork of live-input's heuristic (no search) whose trades are meant to be mutually beneficial, so people see sensible offers and table scores go up.
  - Every offer must leave the partner no worse off by the bot's model of their hand. There is no "fair by scarcity" escape, which house bots accept and people don't.
  - It never asks for a Joker unless it gives more tokens than it asks for, or pays coins.
  - After a partner refuses an offer, it never offers them one that is no better for them that round (no 4 coins, then 2, then 0). Tuned-2 and int-event share this; each has a `never_follows_a_refusal_with_a_worse_offer` test.
  - Offers may ask for two tokens (`max_get`). Candidates are ranked by the bot's gain plus `their_gain_weight` times the partner's.
  - It beats live-input on every bot field. All-tuned-1 tables score more than all-live-input tables. See `tuning/tuned-1/`.
- **`crates/bots/tuned-2`** (seat `tuned-2`): tuned-1 retuned against a stronger field (including tuned-1 and live-input). It values coins more and is choosier about money offers, with the same considerate trades. It is about 2 points ahead of tuned-1 on that field and level elsewhere. See `tuning/tuned-2/`.
- **`crates/bots/int-event`** (seat `int-event`): a fork of tuned-1 with a fixed investing plan.
  - Plan: season ticket, expansion, a "tier 3" event (one that needs one square fewer than the full arena), expansion, then a "tier 4" event (needs the full arena) in the final round.
  - It values hands for both events from round 1. It chases podiums by preferring events that take the lead in best single event. It plays SpecialEvent tokens for a second season ticket when money allows.
  - `examples/diag.rs` breaks down its best event against house's. `tuning/int-event/` holds the tuner and results.
- **`crates/colosseum-cli`**: binary `colosseum`.
  - The `play` command mixes humans and bots at one terminal. Its menus collapse numeric action families (bids, medal counts) into a single prompt.
  - The `sim` command runs bot-only games and reports win rates.
- **`crates/colosseum-session`**: the GUI's JSON API as plain Rust, shared by the axum server and the WebAssembly build. `Api::route(method, path, body) -> (status, json)` handles every endpoint below. `Session` (the `Game`, a seat kind per player, bot agents, a log of event strings) lets bots move until a human seat must act. The host supplies the offered seat kinds, a bot factory and a seed source. `recover.rs` lives here too.
- **`crates/colosseum-wasm`**: `Local`, a `wasm-bindgen` wrapper around `Api` with only the live-input bot. `build.sh` builds `dist/`: the page from `colosseum-web/static`, `web/local.js` and the wasm package. `local.js` sets `window.colosseumLocal`, which `app.js`'s `api()` uses instead of `fetch`, and saves each game to localStorage after every change so reloads keep it. Don't name an exported Rust type `Response` (or any other browser global): the generated loader would shadow it.
- **`crates/colosseum-web`**: binary `colosseum-web`.
  - An axum server that passes every `/api/*` request to `colosseum_session::Api` behind a mutex, and offers every bot. Bots move server-side until a human seat must act.
  - JSON API: `GET /api/config`, `GET /api/seats` (seat kinds offered; the setup menu shows only these), `POST /api/games` (`{seats: ["human","greedy","random"], seed?}`), `GET /api/games/{id}`, `POST /api/games/{id}/actions` (`{action}` in the engine's serde form, e.g. `{"Bid":{"amount":9}}` or `"PassBid"`). Every response is a `View`: public state plus the legal actions with `describe_action` text. Games live only in memory, so to carry one across a restart: `GET /api/games/{id}/save` returns `{seats, seed, history}`, and `POST /api/games/load` (that, plus an optional `id`) replays it. `POST /api/games/{id}/undo` rewinds to before the last human action. `GET /api/games/{id}/step?step=N` returns a read-only `View` of the position after N actions, with a `replay` field (step, total, phase-start marks, the last and next action). The page wires these up: Undo (`U`), Replay (`R`: a step-through bar with move/phase jumps, arrow keys, and "Play from here", which loads the history up to that step as a new game), and a Game menu to save to or load from a JSON file. `POST /api/games/recover` (`{seats, seed, log}`, the log texts after "New game") rebuilds the history of a game from a server that had no save endpoint (`colosseum-session/src/recover.rs`).
  - The page is vanilla HTML/CSS/JS in `static/`, embedded with `include_str!`, so changing it needs a rebuild and server restart. `app.js` redraws everything from each view. Token icons (silhouette SVG paths from game-icons.net, CC BY 3.0, in `static/icons.js`) and colors are presentation, not rules. Don't use emoji for icons. `Assets` serializes as a map like `{"Gladiator": 2}`.
  - Layout ("Stage Bench", chosen from the mockups in `crates/colosseum-web/mockups/`): light tan theme only. A top bar holds the round/phase tracker, Programs (`P`) and Log (`L`). The ring board has square spaces and picks whichever w×h border gives the biggest squares. Its middle is the stage: one function per phase (`investingStage`, `acquiringStage`, `tradingStage`, `producingStage`, `closingStage`) draws that phase's legal actions. Any legal action a stage doesn't draw still gets a plain button. Players sit on a bench underneath: the human's seat is wide, opponents are compact and show their programs on hover.
  - Move playback: responses to `POST /api/games` and `/actions` carry a `trail`, one frame per action applied (the human's own, then each bot move), with the actor, `describe_action` text, the raw `GameEvent`s, its log range and the view after it. `playTrail` in `app.js` shows the frames in turn: a caption in the stage (`spectateStage`), nobles walking square by square, tokens flying between markets and seats, coin/score bubbles, round/phase banners, and each seat's latest move on the bench. Speed (Slow/Normal/Fast/Off) is in the Game menu and stored in localStorage; Space skips.
  - The board takes clicks: noble moves (pick a die, click a noble, click a faded noble on a dashed square) and arena expansion (the `+` squares). Dice show Roman numerals. Trading is a table between two hands: click tokens across, set coins, Offer / Counter.
  - Below 900px wide the stage moves under the ring and the bench stacks.
  - Every human seat plays from the same browser for now; seats in `Session` are where remote players would attach.
- **Online play (milestone 1 done: `colosseum-wasm`)**: see `docs/ONLINE.md` for the Cloudflare design (engine and bots in WebAssembly in the browser, one Durable Object per game relaying moves over WebSockets, finished games stored in D1, shared-password gate) at impresario.arc5.io.
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
