-- One row per finished online game, for later analysis (docs/ONLINE.md).
-- Lists are JSON text. seed + seats + history is a SavedGame: replay any
-- row with Game::replay, or load it in the GUI with "Load from file".
CREATE TABLE games (
  code       TEXT PRIMARY KEY,  -- the invite code
  started_at INTEGER NOT NULL,  -- ms since 1970
  ended_at   INTEGER NOT NULL,  -- ms since 1970
  seed       INTEGER NOT NULL,
  seats      TEXT NOT NULL,     -- ["human","human","live-input"]
  names      TEXT NOT NULL,     -- player names by seat, "" for bots
  scores     TEXT NOT NULL,     -- best_score by seat
  ranking    TEXT NOT NULL,     -- seat indexes, winner first
  moves      INTEGER NOT NULL,
  history    TEXT NOT NULL      -- the engine's Actions, in order
);
CREATE INDEX games_ended ON games (ended_at);
