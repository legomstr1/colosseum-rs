// Serves the page's /api/... requests from WebAssembly instead of a server.
// app.js checks for window.colosseumLocal before falling back to fetch.
// Games live in this tab's memory; each is also saved to localStorage after
// every change so a reload (or an old #id link) picks it up again.
import init, { Local } from './pkg/colosseum_wasm.js';

const KEY = (id) => `colosseum.local.${id}`;

function store(id, saved) {
  try { localStorage.setItem(KEY(id), JSON.stringify(saved)); } catch { /* storage off or full */ }
}

function stored(id) {
  try { return JSON.parse(localStorage.getItem(KEY(id))); } catch { return null; }
}

export async function start() {
  await init();
  const local = new Local();
  const call = (method, path, body) => {
    const r = local.request(method, path, body === undefined ? '' : JSON.stringify(body));
    return { status: r.status, json: JSON.parse(r.body) };
  };
  // Served by the Worker (crates/colosseum-worker), which can host online
  // games. On another static server the button just reports an error.
  window.colosseumCanHost = true;
  const code = new URLSearchParams(location.search).get('g');
  if (code) {
    const { startOnline } = await import('./online.js');
    window.colosseumLocal = startOnline(code.trim().toUpperCase(), call);
    return;
  }
  window.colosseumLocal = (method, path, body) => {
    let r = call(method, path, body);
    // A game from before a reload: rebuild it from localStorage, same id.
    const m = path.match(/^\/api\/games\/([0-9a-f]+)/);
    if (r.status === 404 && m) {
      const saved = stored(m[1]);
      if (saved && call('POST', '/api/games/load', { ...saved, id: m[1] }).status === 200) r = call(method, path, body);
    }
    if (r.status === 200 && method === 'POST' && r.json && typeof r.json.id === 'string') {
      const s = call('GET', `/api/games/${r.json.id}/save`);
      if (s.status === 200) store(r.json.id, s.json);
    }
    return r;
  };
}
