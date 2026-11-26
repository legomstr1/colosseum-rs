# Online play: impresario.arc5.io

Status: milestones 1–3 done. The GUI runs in the browser from WebAssembly, and `crates/colosseum-worker` relays online games between browsers behind a shared password. Absent players can be stood in for by a bot or taken over. It has been tested locally with `wrangler dev`, not yet deployed. Work continues on the `online` branch.

Run it locally:
```bash
crates/colosseum-wasm/build.sh                     # dist/: page + engine (needs the wasm32 target and wasm-bindgen-cli 0.2.130)
echo 'SITE_PASSWORD=pick a passphrase' > crates/colosseum-worker/.dev.vars   # gitignored; without it every request gets 503
cd crates/colosseum-worker && npx wrangler dev     # needs worker-build 0.9 (cargo install worker-build)
                                                   # add --var STANDIN_SECONDS:5 to see stand-ins quickly
```
Open http://127.0.0.1:8787, set seats to Human or Live-input, and press **Create online game**. Share the `?g=CODE` link.

Only the **live-input** bot is offered online.

## Goal
Play remotely with a few friends at `impresario.arc5.io`, on Cloudflare's free tier. Expect at most 2–3 games at once, each lasting 10–20 minutes. The engine is deterministic: `Game::replay(seed, history)` rebuilds any state. So the engine and bots run in the browser as WebAssembly, and the server only relays and stores moves.

## Architecture
- **Workers Static Assets** (`[assets]` in `wrangler.toml`, from `dist/`) serve the existing `static/` GUI plus the WebAssembly bundle. Asset requests are free and never run the Worker, so no separate Pages project is needed.
- **One Worker** handles `/api/*` and `/ws/*`.
  - It routes each game to its own **Durable Object** (SQLite-backed, using the WebSocket Hibernation API).
  - It writes finished games to **D1**.
- **Durable Object per game**
  - Holds `seed`, `seats` (human name or bot kind), seat tokens, and the ordered `history`.
  - Validates each move by running `apply` in WebAssembly, then broadcasts it to every connected client.
  - When the game ends, writes one D1 row and clears its own storage.
- **Clients**
  - Each client runs the engine locally and applies the moves it receives, so no server-side views are needed.
  - The **host browser runs the bots** and submits their moves like any other seat, so the Worker never hits its CPU limit. If the host drops, any other connected client can take over bot duty.

## New / changed crates
- **`crates/colosseum-wasm`** (new): `wasm-bindgen` wrapper.
  - `new_game(seats, seed)`, `apply(action_json)`, `view(seat)`, `legal_actions()`, `describe_action()`, `bot_move(kind)`, `replay(seed, history)`.
  - Reuse the view-building code from `crates/colosseum-web/src/main.rs` (`view_with`, `LegalAction`, `Frame`). Move it into a shared crate or module so the axum server and WebAssembly use the same code.
- **`crates/colosseum-worker`**: the Worker plus the `GameRoom` Durable Object, in Rust with `workers-rs` 0.9, linking the engine directly to check moves. Moves are stored one key per move (`h0000000`, ...). The game is rebuilt by replay after the object hibernates.
- **Bots:** the engine has no OS dependencies (only serde and serde_yml), so it compiles to WebAssembly as is. `crates/bots/live-input/src/search.rs` can use threads.
  - live-input's search is off by default (`budget: 0`, one thread), so it runs on WebAssembly unchanged.
  - Only live-input is built into WebAssembly.
- **`static/app.js`**
  - Replace `fetch('/api/games/...')` calls with calls into the local WebAssembly engine plus a WebSocket.
  - Read `?game=<id>`; keep the seat token in localStorage.
  - Draw from `mySeat`.
  - Animate incoming moves with the existing `playTrail`.
  - Add a password prompt, create-game and join screens, and a "Copy invite link" button.
  - Disable Undo when more than one human has joined.
- Keep `crates/colosseum-web` working for local and offline play.

## Protocol (WebSocket, JSON)
- **HTTP:** `POST /api/online/games {seats}` → `{code}`. `GET /ws/{code}` opens the WebSocket; an unknown code gets 404.
- **Client → server:**
  - `{"join": {"name"?, "seat"?, "token"?}}`: a token that holds a seat gets it back. Otherwise `seat` claims an open human seat, and no seat makes you a spectator. Sending `join` again on the same socket is how a spectator takes a seat.
  - `{"action": {"index", "action"}}`
- **Server → client:**
  - `{"hello": {"seats", "seed", "history", "names", "you": {"seat", "token"}}}`
  - `{"applied": {"index", "action"}}`: sent to everyone, the sender included.
  - `{"seats": {"names", "connected", "bot_runner", "stand_in"}}`: on every join and leave, and when a bot stands in.
  - `{"error": {"message"}}`: sent to the sender only.
- The server checks, before storing and broadcasting a move:
  - `index == history.len()`;
  - the socket holds the seat to act, or that seat is a bot (or stood in) and the socket's seat is `bot_runner`;
  - the engine accepts the move.
- `bot_runner` is the seat of the earliest-joined connected socket. It moves to another player when that browser leaves.
- Seeds are at most 52 bits, because they pass through JavaScript numbers.
- The page: `?g=CODE` makes `local.js` load `online.js`, which stands in for the HTTP API.
  - Moves, the player's own included, arrive through `window.colosseumPush`.
  - Undo is off; Replay works locally.

## Access control (shared password)
- **The password:** the Worker secret `SITE_PASSWORD` (`npx wrangler secret put SITE_PASSWORD`; `.dev.vars` locally). Use a passphrase of 3–4 words. Without it the Worker answers 503 to everything except static files (fail closed).
- **`POST /api/login {password}`:**
  - **Rate limit:** 5 tries per minute per `CF-Connecting-IP`, counted in memory by one `LoginLimiter` Durable Object; over the limit gives 429. The Workers rate-limit binding was skipped because its free-plan availability couldn't be confirmed.
  - **Check:** HMACs of the given and real passwords compared with `verify_slice`, which is constant time. A wrong password gives 401.
  - **Cookie:** on success, `imp_auth=<expiry ms>.<hex HMAC-SHA256(SHA-256("colosseum-auth-v1:" + password), expiry)>`, set with `HttpOnly; SameSite=Lax; Path=/; Max-Age=30 days`, plus `Secure` over https. Changing the password signs everyone out.
- **What the cookie guards:** every other Worker route: `POST /api/online/games`, `GET /api/online/games/{code}` (200 if the game exists, 404 if not) and `/ws/{code}`. Static assets stay public and free.
- **The page:** `online.js` asks `GET /api/online/games/{code}` before opening the WebSocket. 401 shows the password screen (`renderLogin` in `app.js`); so does a 401 from **Create online game**. After logging in, the page carries on where it was.
- Known risks, accepted:
  - The password can spread; fix by changing it.
  - Player names aren't authenticated.
  - Quota exhaustion takes the site down until 00:00 UTC, with no bill.

## Joining and rejoining (user decisions)
- **Create:** the creator picks the seats (human or live-input). The server returns a 6-character game code; the link is `impresario.arc5.io/?g=<code>`. There is no lobby list; you get into a game only through its link or code.
- **Join:** open the link, enter a name (remembered in localStorage), pick an open human seat. The server issues a random 128-bit seat token, stored in localStorage under the game code.
- **Rejoin, same browser:** reconnects automatically with the stored token.
- **Rejoin, other device:** anyone with the password can take over a human seat whose player isn't connected (the **Take over** button on the join screen sends `join {seat, name}`). It issues a new token; the old one stops working, so the old browser gets the join screen. An empty name keeps the old one.
- **Dropouts (auto-bot):**
  - **Countdown:** after every join, leave and accepted move, the object checks whether the seat to act is a human seat with no connected socket. If so, it sets an alarm `STANDIN_SECONDS` later (`[vars]`, 120 by default), unless one is already set. If not, it deletes the alarm, so a stale one can't fire early.
  - **When the alarm fires:** if the seat is still empty, the object sets `stand_in[seat]`, stores it and broadcasts it in `seats`. The bot runner's browser then plays that seat with a live-input bot (`OnlineClient::bot_action`), and the server accepts those moves like any bot seat's.
  - **Shown as:** benches show "Ann · bot" while a bot stands in and "Ann · away" while Ann is disconnected.
  - **Ending it:** when a player rejoins or takes over the seat, the stand-in ends and they play their next decision.
- **Bot duty:** assigned to the creator if connected, otherwise to the connected client that joined earliest. When the client on duty disconnects, the Durable Object passes duty to the next one.
- **Spectators:** connect without a seat and receive the same move stream.

## Storage
- **D1 table `games`:** `id, started_at, ended_at, seed, seats_json, scores_json, history_json`. About 10–20 KB per game, so roughly 33k games per 500 MB database.
- **Past games page:** lists rows; opening one loads the existing Replay viewer through `replay()`.

## Free-tier budget (per 20-min, 5-player game)
- **Durable Object requests:** about 50 (WebSocket messages count 20 to 1).
- **Durable Object duration:** at most 1,200 s, out of about 104,000 s per day.
- **D1 writes:** 1 row per game.
- **Daily ceiling:** about 86–250 games per day. 2–3 concurrent games is fine.

## Deploy
- `crates/colosseum-worker/wrangler.toml`: the Worker with static assets from `dist/`, the `GAMES` Durable Object binding (SQLite-backed), and, later, D1 and a custom domain route for `impresario.arc5.io`.
- DNS: `impresario` CNAME in the arc5.io zone (already on Cloudflare).
- Build: `crates/colosseum-wasm/build.sh`, then `npx wrangler deploy` from `crates/colosseum-worker`.

## Milestones
1. ✅ `colosseum-wasm` builds; the local GUI runs fully in the browser (hotseat, bots in the browser) with no server. The API moved into `crates/colosseum-session`, shared by the axum server and WebAssembly.
2. ✅ Worker plus Durable Object relay with seat tokens; two browsers play one game. Tested with two Playwright browser contexts through `wrangler dev`:
   - a full game, with both browsers ending in the same state;
   - reload rejoins the same seat;
   - out-of-turn, stale-index and spectator moves are rejected;
   - games survive a restart;
   - bot duty moves to another player when the runner leaves.
3. ✅ Password gate, invite links, claiming and reclaiming seats, auto-bot after timeout, bot duty passed to another client. Tested through `wrangler dev`:
   - **Gate, raw HTTP and WebSocket:** without the cookie, create, exists and the WebSocket all get 401. A wrong password gets 401, and the 6th try in a minute gets 429. A tampered or expired cookie gets 401. Changing the password signs old cookies out. With no password set, the server answers 503.
   - **Three Playwright browsers:**
     - A logs in through the password screen (a wrong password first) and creates the game; B logs in and joins.
     - B closes: A sees "Bo · away", then "Bo · bot" about 3 s into B's turn (`STANDIN_SECONDS:3`), and the game keeps going.
     - C logs in and uses **Take over** on B's seat.
     - B returns: its token has been dropped, so it gets the join screen.
     - A and C finish the game in the same state.
4. D1 archive plus past-games page.
5. Deploy to impresario.arc5.io.

## Verification
- **Engine:** a test that compiles and runs on `wasm32` (`wasm-pack test --node`) replaying a recorded bot game and checking that the final scores match native.
- **Relay:** `wrangler dev` with two browser profiles. Check that a move with the wrong token is rejected and that a reload gets the seat back.
- **Budget:** confirm the Durable Object and D1 dashboards stay far below the free limits after a game night.
