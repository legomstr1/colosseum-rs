# Online play: impresario.arc5.io

Status: milestone 1 done (the GUI runs in the browser from WebAssembly: `crates/colosseum-wasm/build.sh`). Work continues on the `online` branch.

Only the **live-input** bot is offered online.

## Goal
Play remotely with a few friends at `impresario.arc5.io`, on Cloudflare's free tier. Expect at most 2–3 games at once, each lasting 10–20 minutes. The engine is deterministic: `Game::replay(seed, history)` rebuilds any state. So the engine and bots run in the browser as WebAssembly, and the server only relays and stores moves.

## Architecture
- **Cloudflare Pages** (`impresario.arc5.io`) serves the existing `static/` GUI plus a new WebAssembly bundle built from the engine and the bots.
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
- **`crates/colosseum-worker`** (new): a Worker plus Durable Object, written in Rust with `workers-rs` so it can link the engine directly. Fallback: a thin TypeScript Worker that imports the WebAssembly engine.
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
- **Client → server:**
  - `{"join": {"name", "seat?", "token?"}}`
  - `{"action": {"seat", "token", "action"}}`
- **Server → client:**
  - `{"hello": {"seed", "seats", "history", "you": {"seat", "token"}}}`
  - `{"applied": {"index", "seat", "action"}}`
  - `{"error": "..."}`
  - `{"seats": [...]}`
- The server checks: the token matches the seat, it is that seat's turn, the action is legal, and `index == history.len()`.

## Access control (shared password)
- Store the password as a Worker secret (`SITE_PASSWORD`). The user picks a passphrase of 3–4 words.
- `POST /api/login {password}`:
  - Compare in constant time.
  - On success, set cookie `imp_auth`: HttpOnly, Secure, SameSite=Lax, 30 days. Its value is an HMAC of the expiry, keyed by a key derived from the password, so changing the password logs everyone out.
  - Rate-limit to 5 attempts per minute per IP (Workers rate-limiting binding, or a counter in a Durable Object).
- Only `/api/*` and `/ws/*` check the cookie. Static Pages assets stay public, so they cost no Function requests.
- The GUI shows a password prompt whenever an API call returns 401.
- Known risks, accepted:
  - The password can spread; fix by changing it.
  - Player names aren't authenticated.
  - Quota exhaustion takes the site down until 00:00 UTC, with no bill.

## Joining and rejoining (user decisions)
- **Create:** the creator picks the seats (human or bot kind). The server returns a 4–6 character game code and the link `impresario.arc5.io/g/<code>`. There is no lobby list; you get into a game only through its link or code.
- **Join:** open the link, enter a name (remembered in localStorage), pick an open human seat. The server issues a random 128-bit seat token, stored in localStorage under the game code.
- **Rejoin, same browser:** reconnects automatically with the stored token.
- **Rejoin, other device:** anyone with the password can claim a human seat whose player is not currently connected. Claiming issues a new token, and any old token stops working.
- **Dropouts (auto-bot):**
  - When the seat to act has had no connected player for 2 minutes, a Durable Object alarm marks it `bot_controlled` with the `live-input` bot and broadcasts that.
  - The client on bot duty plays that seat until a player reconnects or claims it. Control then returns to the player at their next decision.
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
- `wrangler` config: a Pages project for the GUI; Worker routes `impresario.arc5.io/api/*` and `/ws/*`; Durable Object binding; D1 binding.
- DNS: `impresario` CNAME in the arc5.io zone (already on Cloudflare).
- Build: `wasm-pack build crates/colosseum-wasm --target web`, then copy the output into the Pages output alongside `static/`.

## Milestones
1. ✅ `colosseum-wasm` builds; the local GUI runs fully in the browser (hotseat, bots in the browser) with no server. The API moved into `crates/colosseum-session`, shared by the axum server and WebAssembly.
2. Worker plus Durable Object relay with seat tokens; two browsers play one game.
3. Password gate, invite links, claiming and reclaiming seats, auto-bot after timeout, bot duty passed to another client.
4. D1 archive plus past-games page.
5. Deploy to impresario.arc5.io.

## Verification
- **Engine:** a test that compiles and runs on `wasm32` (`wasm-pack test --node`) replaying a recorded bot game and checking that the final scores match native.
- **Relay:** `wrangler dev` with two browser profiles. Check that a move with the wrong token is rejected and that a reload gets the seat back.
- **Budget:** confirm the Durable Object and D1 dashboards stay far below the free limits after a game night.
