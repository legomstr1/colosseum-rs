// Online play: one game shared with other browsers through the Worker's
// Durable Object (crates/colosseum-worker). Loaded by local.js when the page
// URL has ?g=CODE.
//
// The server only orders and checks moves. This browser runs the engine
// itself (OnlineClient) and plays every accepted move, its own included,
// through app.js's colosseumPush. The browser the server names as bot
// runner also sends the bots' moves.
import { OnlineClient } from './pkg/colosseum_wasm.js';

const TOKEN = (code) => `colosseum.online.${code}`;

function read(key) {
  try { return localStorage.getItem(key); } catch { return null; }
}
function write(key, value) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch { /* storage off */ }
}

// `local(method, path, body)` is the in-browser API from local.js, used for
// the config, the seat list and step-through views.
export function startOnline(code, local) {
  let ws = null;
  let client = null;
  let you = { seat: null, token: read(TOKEN(code)) };
  let watching = false;
  let everOpened = false;
  let retry = 0;
  let botRunner = null;
  let botPending = null;
  let seats = [];
  let seed = 0;

  const o = {
    code,
    state: 'connecting', // connecting | login | join | playing | missing
    seats: [],
    names: [],
    connected: [],
    standIn: [],
    start,
    // Also takes over a seat whose player isn't connected.
    join(seat, name) {
      name = (name || '').trim();
      write('colosseum.name', name);
      send({ join: { seat, name } });
    },
    watch() {
      watching = true;
      o.state = 'playing';
      show();
    },
  };
  window.colosseumOnline = o;

  function send(msg) {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
    else window.colosseumToast("Not connected to the game; retrying…");
  }

  function show() {
    window.colosseumShow(client && o.state === 'playing' ? JSON.parse(client.view()) : null);
  }

  // Ask first, so a missing password and a wrong code look different: the
  // WebSocket upgrade can only fail without saying why.
  async function start() {
    if (o.state === 'login' || o.state === 'missing') o.state = 'connecting';
    let status;
    try {
      status = (await fetch(`/api/online/games/${code}`)).status;
    } catch {
      status = 0;
    }
    if (status === 401) o.state = 'login';
    else if (status === 404) o.state = 'missing';
    else if (status === 200) { connect(); return; } else {
      setTimeout(start, 3000);
      return;
    }
    window.colosseumShow(null);
  }

  function connect() {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    ws = new WebSocket(`${proto}//${location.host}/ws/${code}`);
    ws.onopen = () => {
      everOpened = true;
      retry = 0;
      send({ join: you.token ? { token: you.token } : {} });
    };
    ws.onmessage = (e) => receive(JSON.parse(e.data));
    ws.onclose = () => {
      ws = null;
      // Ask again before retrying: the game may be gone or the login expired.
      const wait = everOpened ? Math.min(15000, 1000 * 2 ** retry++) : 3000;
      setTimeout(start, wait);
    };
  }

  function receive(msg) {
    if (msg.hello) hello(msg.hello);
    else if (msg.applied) applied(msg.applied);
    else if (msg.seats) seatsUpdate(msg.seats);
    else if (msg.error) window.colosseumToast(msg.error.message);
  }

  function hello(h) {
    seats = h.seats;
    seed = h.seed;
    o.seats = h.seats;
    o.names = h.names;
    if (o.connected.length !== seats.length) o.connected = seats.map(() => false);
    if (you.token && h.you.seat === null) {
      // The stored token no longer holds a seat here.
      write(TOKEN(code), null);
    }
    you = h.you;
    if (you.token) write(TOKEN(code), you.token);
    client = new OnlineClient(code, JSON.stringify(seats), seed, JSON.stringify(h.history), JSON.stringify(shownNames()), you.seat ?? -1);
    if (o.standIn.length === seats.length) client.set_stand_ins(JSON.stringify(o.standIn));
    // A finished game has no seats to take: just show it.
    if (client.current_player() < 0) watching = true;
    o.state = you.seat !== null || watching ? 'playing' : 'join';
    show();
    maybeBot();
  }

  function applied({ index, action }) {
    if (!client) return;
    const len = client.history_len();
    if (index < len) return; // already have it
    if (index > len) {
      // Missed a move: reconnecting resends the whole game.
      ws.close();
      return;
    }
    let next;
    try {
      next = JSON.parse(client.apply(JSON.stringify(action)));
    } catch (e) {
      window.colosseumToast(`Out of step with the server (${e}); reconnecting`);
      ws.close();
      return;
    }
    if (o.state === 'playing') window.colosseumPush(next);
    maybeBot();
  }

  // Names as the bench shows them: "Ann · bot" while a bot stands in,
  // "Ann · away" while Ann isn't connected.
  function shownNames() {
    const over = client && client.current_player() < 0;
    return o.names.map((n, i) => {
      if (!n || over) return n;
      if (o.standIn[i]) return `${n} · bot`;
      if (o.connected.length && !o.connected[i] && seats[i] === 'human') return `${n} · away`;
      return n;
    });
  }

  function seatsUpdate(s) {
    o.names = s.names;
    o.connected = s.connected;
    o.standIn = s.stand_in;
    botRunner = s.bot_runner;
    const names = shownNames();
    if (client) {
      client.set_names(JSON.stringify(names));
      client.set_stand_ins(JSON.stringify(s.stand_in));
    }
    if (o.state === 'join') window.colosseumShow(null);
    else window.colosseumPatch((v) => { if (v.online) v.online.names = names; });
    maybeBot();
  }

  // The bot runner sends the bots' moves, one at a time, each after the
  // previous one comes back from the server.
  function maybeBot() {
    if (!client || you.seat === null || botRunner !== you.seat || !client.bot_to_act()) return;
    const at = client.history_len();
    if (botPending === at) return;
    botPending = at;
    setTimeout(() => {
      if (client.history_len() !== at || !client.bot_to_act()) return;
      const action = client.bot_action();
      if (action) send({ action: { index: at, action: JSON.parse(action) } });
    }, 0);
  }

  const reply = (status, json) => ({ status, json });
  const notOnline = () => reply(409, { error: 'Not available in an online game' });

  // Stands in for the HTTP API while this page plays an online game.
  return (method, path, body) => {
    const [route, query] = path.split('?');
    const parts = route.split('/').filter(Boolean); // api, games, code, ...
    if (parts[1] !== 'games') return local(method, path, body);
    if (parts.length === 3 && method === 'GET' && client) return reply(200, JSON.parse(client.view()));
    const what = parts[3];
    if (what === 'actions' && method === 'POST' && client) {
      send({ action: { index: client.history_len(), action: body.action } });
      return reply(200, null);
    }
    if (what === 'save' && client) return reply(200, JSON.parse(client.save()));
    if (what === 'step' && client) {
      // Replay the moves in a scratch game with no bots, then show it with
      // the real seats and names.
      const saved = JSON.parse(client.save());
      const id = `replay-${code}`;
      const loaded = local('POST', '/api/games/load', { ...saved, seats: saved.seats.map(() => 'human'), id });
      if (loaded.status !== 200) return loaded;
      const r = local('GET', `/api/games/${id}/step?${query}`);
      if (r.status === 200) {
        r.json.id = code;
        r.json.seats = saved.seats;
        r.json.online = JSON.parse(client.view()).online;
      }
      return r;
    }
    if (what === 'undo') return reply(409, { error: 'Undo is off in online games' });
    return notOnline();
  };
}
