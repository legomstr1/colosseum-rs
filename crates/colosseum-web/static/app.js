// Colosseum web client ("Stage Bench" layout). Talks to the JSON API in
// colosseum-web and redraws the page from each view the server returns.
//
// Layout: a top bar (round and phase tracker, programs, log), a ring board
// with square spaces whose shape adapts to the window, a stage in the
// middle of the ring that shows only the current phase's choices, and a
// bench of player seats underneath. The board itself takes clicks for noble
// moves and arena expansion.
'use strict';

// ---------- presentation theme (not game rules) ----------
const TOKENS = {
  Gladiator:    { bg: '#fcd4d4', fg: '#b3343a' },
  Ship:         { bg: '#cdeef0', fg: '#1b7f88' },
  Musician:     { bg: '#dcf5d2', fg: '#4c9a2a' },
  Comedian:     { bg: '#fdeca0', fg: '#8f6c00' },
  Priest:       { bg: '#ffffff', fg: '#5b616e' },
  Horse:        { bg: '#ead9c6', fg: '#85552a' },
  Lion:         { bg: '#ffd8b5', fg: '#c25a0c' },
  Chariot:      { bg: '#e6dafc', fg: '#6a45c2' },
  Cage:         { bg: '#eceef2', fg: '#7a808c' },
  Torch:        { bg: '#9aa0aa', fg: '#2f333a' },
  Scenery:      { bg: '#cfe2fb', fg: '#2c66b3' },
  Decoration:   { bg: '#dde6bd', fg: '#5c6a1c' },
  Joker:        { bg: '#ffffff', fg: '#3b3f48' },
  Emperor:      { bg: '#fff1b8', fg: '#94700a' },
  SpecialEvent: { bg: '#efe6ff', fg: '#7a4fc0' },
};
const NOBLES = {
  Emperor: { bg: '#ffd6d2', fg: '#a8241a' },
  Consul:  { bg: '#d6e6ff', fg: '#1f57b8' },
  Senator: { bg: '#ffeaa0', fg: '#7a5a00' },
};
// Tokens that can't change hands in a trade.
const UNTRADEABLE = new Set(['Emperor', 'SpecialEvent']);
const SEAT_LABEL = {
  human: 'Human', oracle: 'Oracle', maestro: 'Maestro', chameleon: 'Chameleon', trader: 'Trader', planner: 'Planner',
  auctioneer: 'Auctioneer', showman: 'Showman', rollout: 'Rollout (slow)', greedy: 'Greedy', random: 'Random',
};
const PHASES = ['Investing', 'Acquiring', 'Trading', 'Producing', 'Closing'];
const ROMAN = ['', 'I', 'II', 'III', 'IV', 'V', 'VI', 'VII', 'VIII', 'IX', 'X', 'XI', 'XII'];

// ---------- state ----------
let config = null;
let view = null;
let ui = freshUi();
let setup = { seats: ['human', 'oracle', 'oracle'], seed: '' };
let handlers = [];
let showPrograms = false;
let showLog = false;
let logSeen = 0; // log entries before this index have been seen in the drawer
let ringObserver = null;

function freshUi() {
  return { die: null, noble: null, medal: false, market: null, bid: {}, medals: null, trade: null };
}

// ---------- helpers ----------
const $app = document.getElementById('app');
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const icon = (name) => `<svg class="ic" viewBox="0 0 512 512" aria-hidden="true"><path fill="currentColor" d="${ICON_PATHS[name]}"/></svg>`;
const CHECK = '<svg class="ic" viewBox="0 0 512 512" aria-hidden="true"><path d="M96 272l112 112L416 144" fill="none" stroke="currentColor" stroke-width="56" stroke-linecap="round" stroke-linejoin="round"/></svg>';
const on = (fn) => { handlers.push(fn); return handlers.length - 1; };
const kind = (a) => (typeof a === 'string' ? a : Object.keys(a)[0]);
const body = (a) => (typeof a === 'string' ? {} : a[kind(a)]);
const phaseName = (p) => (typeof p === 'string' ? p : Object.keys(p)[0]);
const phaseBody = (p) => (typeof p === 'string' ? {} : p[phaseName(p)]);
const pName = (i) => `P${i + 1}`;
const pColor = (i) => `var(--p${i % 5})`;
const rules = () => config.rules;
const boardSize = () => rules().board.size;
const event = (id) => config.events.find((e) => e.id === id);
const legalOf = (...kinds) => view.legal.filter((l) => kinds.includes(kind(l.action)));
const isHuman = (i) => view.seats[i] === 'human';
const n = () => view.players.length;

// The seat the screen is drawn for: the human to act, else the first human.
function me() {
  if (view.human_to_act && view.current_player !== null) return view.current_player;
  const h = view.seats.indexOf('human');
  return h >= 0 ? h : 0;
}
const youOr = (i) => (i === me() && isHuman(i) ? 'You' : pName(i));

function countList(list) {
  const out = {};
  for (const a of list) out[a] = (out[a] || 0) + 1;
  return out;
}
// Assets maps in a stable order (the order of TOKENS).
function entries(map) {
  return Object.keys(TOKENS).filter((a) => map[a] > 0).map((a) => [a, map[a]]);
}
function subtract(have, minus) {
  const out = { ...have };
  for (const [a, k] of Object.entries(minus)) out[a] = (out[a] || 0) - k;
  return out;
}
const sumMap = (m) => Object.values(m).reduce((s, x) => s + x, 0);

// One token tile. Player tokens are always drawn one tile each.
function tok(asset, size = '') {
  const t = TOKENS[asset] || { bg: '#eee', fg: '#555' };
  const cls = ['tok', size, asset === 'Joker' ? 'joker' : '', t.bg === '#ffffff' ? 'white' : ''].join(' ');
  return `<span class="${cls}" style="--c:${t.bg};color:${t.fg}" title="${asset}">${icon(asset)}</span>`;
}
// A player's tokens: one column per type; star types get a gold outline.
function seatToks(i, size = 'xs') {
  const list = entries(view.players[i].assets);
  if (!list.length) return '';
  const starOf = new Set(view.stars.filter(([, h]) => h === i).map(([a]) => a));
  return `<span class="toks">${list.map(([a, k]) => `<span class="tcol ${starOf.has(a) ? 'starcol' : ''}" title="${a}${starOf.has(a) ? ' (star)' : ''}">${
    Array.from({ length: k }, () => tok(a, size)).join('')}</span>`).join('')}</span>`;
}
// An event's needs as a row; with `have`, tokens the holder lacks are faded.
function needRow(ev, have = null, size = '') {
  const left = have ? { ...have } : null;
  return `<span class="mrow need">${ev.requirements.map((a) => {
    const ok = !left || left[a] > 0;
    if (left && ok) left[a] -= 1;
    return tok(a, `${size} ${ok ? '' : 'miss'}`);
  }).join('')}</span>`;
}
function matched(ev, have) {
  const left = { ...have };
  let k = 0;
  for (const a of ev.requirements) if (left[a] > 0) { left[a] -= 1; k += 1; }
  return k;
}
// Tokens still missing for an event, counting Jokers as stand-ins.
function missingFor(ev, have) {
  return Math.max(0, ev.requirements.length - matched(ev, have) - (have.Joker || 0));
}
// Spectators by tokens missing: all present, one missing, two missing...
function falloff(ev, cur = null, mini = false) {
  const cells = [ev.base_score, ...ev.penalty_scores];
  return `<span class="falloff ${mini ? 'mini' : ''}">${Array.from({ length: mini ? cells.length : 7 }, (_, i) => (i < cells.length
    ? `<span class="${i ? '' : 'base'} ${i === cur ? 'cur' : ''}" title="${i} missing: ${cells[i]} spectators">${cells[i]}</span>`
    : '<span class="none"></span>')).join('')}</span>`;
}
// The arena an event needs, as a broken stadium outline.
function arenaIcon(ev) {
  const k = rules().arena.squares_required_by_event_size[ev.size];
  let x = 8;
  let d = 'M8 2A6 6 0 0 0 8 14';
  for (let i = 2; i < k; i++) { x += 3; d += `M${x} 2h6M${x} 14h6`; x += 6; }
  x += 3;
  d += `M${x} 2A6 6 0 0 1 ${x} 14`;
  return `<svg class="arena-ic" viewBox="0 0 ${x + 8} 16" style="width:${(x + 8) * 1.05}px"><title>Needs an arena of ${k}</title><path d="${d}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`;
}
const repeatIcon = (name, k, title) => (k ? `<span class="icons-n" title="${k} ${title}">${Array.from({ length: k }, () => icon(name)).join('')}</span>` : '');
const roman = (face) => (Array.isArray(face) ? `${ROMAN[Math.min(...face)]}–${ROMAN[Math.max(...face)]}` : ROMAN[face] || String(face));
const spectatorsIn = (text) => Number((text.match(/(\d+) spectators/) || [])[1] || 0);

function toast(lines) {
  const el = document.getElementById('toast');
  const list = Array.isArray(lines) ? lines : [lines];
  el.innerHTML = list.map((l) => `<span>${esc(l)}</span>`).join('');
  el.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => { el.hidden = true; }, 3800);
}

async function api(path, data) {
  const res = await fetch(path, data === undefined ? {} : {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(data),
  });
  const json = await res.json();
  if (!res.ok) throw new Error(json.error || res.statusText);
  return json;
}

async function act(action) {
  try {
    const before = view.log.length;
    view = await api(`/api/games/${view.id}/actions`, { action });
    ui = freshUi();
    showPrograms = false;
    // What happened, including the bots' moves, in a short toast.
    const fresh = view.log.slice(before).filter((e) => e.kind === 'event').map((e) => e.text);
    if (fresh.length) toast(fresh.length > 4 ? [...fresh.slice(0, 3), `…and ${fresh.length - 3} more (log: L)`] : fresh);
    render();
  } catch (e) {
    toast(e.message);
  }
}

// ---------- setup screen ----------
function renderSetup() {
  const counts = Object.keys(rules().starting_positions).map(Number);
  const [min, max] = [Math.min(...counts), Math.max(...counts)];
  const seats = setup.seats.map((s, i) => {
    const opts = Object.keys(SEAT_LABEL).map((k) =>
      `<option value="${k}" ${s === k ? 'selected' : ''}>${SEAT_LABEL[k]}${k === 'human' ? '' : ' bot'}</option>`).join('');
    return `<div class="seat-row"><span class="dot" style="--pc:${pColor(i)}"></span><b style="width:28px">${pName(i)}</b><select class="input seat-kind" data-seat="${i}">${opts}</select></div>`;
  }).join('');
  return `
    <div class="setup">
      <h1>Colosseum</h1>
      <div class="muted">Produce the most spectacular event in Rome. Everyone marked Human plays from this screen.</div>
      <div class="card"><h3>Players</h3>
        <div style="display:flex;flex-direction:column;gap:10px">${seats}</div>
        <div class="btns" style="margin-top:12px;justify-content:flex-start">
          <button class="btn small" ${setup.seats.length >= max ? 'disabled' : ''} data-h="${on(() => { setup.seats.push('oracle'); render(); })}">Add player</button>
          <button class="btn small" ${setup.seats.length <= min ? 'disabled' : ''} data-h="${on(() => { setup.seats.pop(); render(); })}">Remove player</button>
        </div></div>
      <div class="row"><input class="input" id="seed" placeholder="Seed (optional)" value="${esc(setup.seed)}" style="width:170px">
        <span class="spacer"></span><button class="btn primary big" data-h="${on(startGame)}">Start game</button></div>
      <div class="hint">Icons by Lorc, Delapouite and Cathelineau from <a href="https://game-icons.net" target="_blank" rel="noopener">game-icons.net</a>, under <a href="https://creativecommons.org/licenses/by/3.0/" target="_blank" rel="noopener">CC BY 3.0</a>.</div>
    </div>`;
}

async function startGame() {
  setup.seed = document.getElementById('seed').value.trim();
  const req = { seats: setup.seats };
  if (setup.seed !== '') req.seed = Number(setup.seed);
  try {
    view = await api('/api/games', req);
    ui = freshUi();
    logSeen = 0;
    location.hash = view.id;
    render();
  } catch (e) {
    toast(e.message);
  }
}

// ---------- top bar ----------
function tracker() {
  const ph = phaseName(view.phase);
  const now = view.game_over ? PHASES.length : PHASES.indexOf(ph);
  const round = Math.min(view.round, view.num_rounds);
  const pips = Array.from({ length: view.num_rounds }, (_, i) =>
    `<span class="${i + 1 < round || view.game_over ? 'done' : i + 1 === round ? 'now' : ''}"></span>`).join('');
  const phases = PHASES.map((p, i) => `<span class="ph ${i < now ? 'done' : i === now ? 'now' : ''}"><i></i>${p}</span>`).join('');
  return `<div class="tracker"><div class="rnd"><span class="big">${view.game_over ? 'Game over' : `Round ${round}<small> / ${view.num_rounds}</small>`}</span><span class="pips">${pips}</span></div>
    <div class="phases">${phases}</div></div>`;
}
function topBar() {
  const unseen = view.log.slice(logSeen).filter((e) => e.kind === 'event').length;
  return `<div class="bar"><span class="brand">Colosseum</span>${tracker()}<span class="spacer"></span>
    <button class="iconbtn" data-h="${on(() => { showPrograms = true; render(); })}" title="Programs for sale">Programs <kbd>P</kbd></button>
    <button class="iconbtn" data-h="${on(toggleLog)}" title="Log">Log ${unseen && !showLog ? `<span class="badge">${unseen > 99 ? '99+' : unseen}</span>` : ''}<kbd>L</kbd></button>
    <button class="btn small ghost" data-h="${on(newGame)}" title="Seed ${view.seed}">New game</button></div>`;
}
function toggleLog() {
  showLog = !showLog;
  if (!showLog) logSeen = view.log.length;
  render();
}

// ---------- ring board ----------
// Every rectangle whose border has as many squares as the board, kept to
// reasonable proportions; the one with the biggest squares wins.
function fitShape(W, H, g) {
  const sum = boardSize() / 2 + 2;
  const minSide = Math.max(4, Math.round(sum * 0.3));
  let best = null;
  for (let w = minSide; w <= sum - minSide; w++) {
    const h = sum - w;
    const s = Math.floor(Math.min((W - (w - 1) * g) / w, (H - (h - 1) * g) / h));
    if (!best || s > best.s || (s === best.s && w > best.w)) best = { w, h, s };
  }
  best.s = Math.max(24, Math.min(best.s, 130));
  return best;
}
// Square i on a w×h border, clockwise from the top-left corner.
function rectPos(i, w, h) {
  if (i < w - 1) return [0, i];
  if (i < w - 1 + h - 1) return [i - (w - 1), w - 1];
  if (i < 2 * (w - 1) + h - 1) return [h - 1, (w - 1) - (i - (w - 1) - (h - 1))];
  return [(h - 1) - (i - 2 * (w - 1) - (h - 1)), 0];
}
function arenaOwner(sq) {
  const N = boardSize();
  for (const p of view.players) {
    if ((sq - p.arena.start + N) % N < p.arena.len) return p.id;
  }
  return null;
}
function expansionSquare(arena, side) {
  const N = boardSize();
  return side === 'Left' ? (arena.start - 1 + N) % N : (arena.start + arena.len) % N;
}

// The dice choice in force: the selected die, else the first one with moves.
function activeDie() {
  const moves = legalOf('MoveNoble');
  const both = legalOf('MoveNobleCombined').length > 0;
  const dice = [...new Set(moves.map((l) => body(l.action).die))];
  if (ui.die === 'both' && both) return 'both';
  if (ui.die !== null && dice.includes(ui.die)) return ui.die;
  return dice.length ? dice[0] : both ? 'both' : null;
}
// Where nobles can go now: {square: [{noble, label, action}]}, plus the nobles that can move.
function boardMoves() {
  const N = boardSize();
  const targets = {};
  const movable = new Set();
  const add = (noble, to, label, action) => {
    movable.add(noble);
    if (ui.noble !== null && ui.noble !== noble) return;
    (targets[to] ||= []).push({ noble, label, action });
  };
  if (!view.human_to_act) return { targets, movable };
  if (ui.medal) {
    for (const l of legalOf('MedalMoveNoble')) {
      const b = body(l.action);
      const pos = view.nobles[b.noble].position;
      add(b.noble, ((pos + b.delta) % N + N) % N, `${b.delta > 0 ? '+' : ''}${b.delta}`, l.action);
    }
    return { targets, movable };
  }
  const die = activeDie();
  for (const l of legalOf('MoveNoble', 'MoveNobleCombined')) {
    const b = body(l.action);
    const match = die === 'both' ? kind(l.action) === 'MoveNobleCombined' : kind(l.action) === 'MoveNoble' && b.die === die;
    if (match) add(b.noble, (view.nobles[b.noble].position + b.distance) % N, `+${b.distance}`, l.action);
  }
  return { targets, movable };
}

function ringHtml(W, H, stage) {
  const g = 5;
  const { w, h, s } = fitShape(W, H, g);
  const N = boardSize();
  const rest = new Set(rules().noble_positions.map(([sq]) => sq));
  const { targets, movable } = boardMoves();
  const grow = {};
  for (const l of legalOf('ExpandArena')) grow[expansionSquare(view.players[me()].arena, body(l.action).side)] = l;
  const at = {};
  view.nobles.forEach((nb, i) => (at[nb.position] ||= []).push(i));
  let cells = '';
  for (let sq = 0; sq < N; sq++) {
    const [r, c] = rectPos(sq, w, h);
    const owner = arenaOwner(sq);
    const tg = targets[sq];
    const cls = ['sq', rest.has(sq) ? 'rest' : '', owner !== null ? 'arena' : '', tg ? 'target' : ''].join(' ');
    const title = `Square ${sq}${owner !== null ? ` · ${pName(owner)}'s arena` : ''}${rest.has(sq) ? ' · resting place' : ''}`;
    const nobles = (at[sq] || []).map((i) => {
      const nb = view.nobles[i];
      const style = `--nc:${NOBLES[nb.kind].bg};color:${NOBLES[nb.kind].fg}`;
      return movable.has(i)
        ? `<button class="noble ${ui.noble === i ? 'sel' : ''}" style="${style}" title="${nb.kind}" data-h="${on(() => { ui.noble = ui.noble === i ? null : i; drawRing(); })}">${icon(nb.kind)}</button>`
        : `<span class="noble" style="${style}" title="${nb.kind}">${icon(nb.kind)}</span>`;
    }).join('');
    const ghosts = (tg || []).map((t) => {
      const nb = view.nobles[t.noble];
      return `<button class="gmove" title="Move ${nb.kind} ${t.label} to ${sq}" data-h="${on(() => act(t.action))}"><span class="noble" style="--nc:${NOBLES[nb.kind].bg};color:${NOBLES[nb.kind].fg}">${icon(nb.kind)}</span>${tg.length > 1 || ui.noble === null ? '' : `<span class="d">${t.label}</span>`}</button>`;
    }).join('');
    const growBtn = grow[sq] ? `<button class="grow" style="--pc:${pColor(me())}" title="${esc(grow[sq].text)}" data-h="${on(() => act(grow[sq].action))}">+</button>` : '';
    cells += `<div class="${cls}" style="grid-row:${r + 1};grid-column:${c + 1};${owner !== null ? `--pc:${pColor(owner)}` : ''}" title="${title}"><span class="num">${sq}</span>${nobles ? `<span class="nobles">${nobles}</span>` : ''}${ghosts ? `<span class="ghosts">${ghosts}</span>` : ''}${growBtn}</div>`;
  }
  const inside = stage === null ? '' : `<div class="stage-wrap" style="grid-row:2/${h};grid-column:2/${w}">${stage}</div>`;
  return `<div class="ring" style="--s:${s}px;grid-template-columns:repeat(${w},${s}px);grid-template-rows:repeat(${h},${s}px)">${cells}${inside}</div>`;
}

// Draws the ring (and the stage) into #ring at whatever size it has.
function drawRing() {
  const el = document.getElementById('ring');
  if (!el || !view) return;
  const narrow = window.innerWidth < 900;
  const r = el.getBoundingClientRect();
  const stage = stageHtml();
  if (narrow) {
    const w = Math.min(r.width, document.documentElement.clientWidth - 24);
    el.innerHTML = `${ringHtml(w, w, null)}<div class="stage-out" style="--s:60px">${stage}</div>`;
  } else {
    el.innerHTML = ringHtml(r.width, r.height, stage);
  }
}

// ---------- the stage: one phase at a time ----------
function stageHtml() {
  const cur = view.current_player;
  const pc = pColor(cur ?? me());
  if (view.game_over) return `<div class="stage idle">${gameOverInner()}</div>`;
  if (!view.human_to_act) {
    return `<div class="stage idle"><div class="thinking"><span class="who-chip" style="--pc:${pc}">${pName(cur)}</span><span class="dots"><i></i><i></i><i></i></span></div></div>`;
  }
  const ph = phaseName(view.phase);
  const handled = new Set(['ExchangeMedalForCoins', 'ExchangeSpecialEventForMedal']);
  const take = (...kinds) => { kinds.forEach((k) => handled.add(k)); return legalOf(...kinds); };
  let inner = '';
  let cls = '';
  if (ph === 'Investing') inner = investingStage(take);
  else if (ph === 'Acquiring') inner = acquiringStage(take);
  else if (ph === 'Trading') { inner = tradingStage(take); cls = 'trade'; }
  else if (ph === 'Producing') inner = producingStage(take);
  else if (ph === 'Closing') inner = closingStage(take);
  // Anything this stage doesn't draw still gets a plain button.
  const rest = view.legal.filter((l) => !handled.has(kind(l.action)));
  const extra = rest.length ? `<div class="st-foot">${rest.map((l) => `<button class="btn small" data-h="${on(() => act(l.action))}">${esc(l.text)}</button>`).join('')}</div>` : '';
  return `<div class="stage ${cls}" style="--pc:${pc}">${inner}${extra}</div>`;
}

// Investing: one tile per kind of investment, priced; programs open the list.
function investingStage(take) {
  const inv = rules().investing;
  const buy = take('BuyProgram');
  const expand = take('ExpandArena');
  const ticket = take('BuySeasonTicket');
  const loge = take('BuildLoge');
  const special = take('PlaySpecialEvent');
  const medals = take('SpendMedalsForInvestment');
  const end = take('EndInvesting');
  const cheapest = Math.min(...view.programs_for_sale.map((id) => event(id).cost));
  const tile = (inner, label, price, list, onClick) => {
    const ok = list.length > 0;
    const h = ok ? on(onClick || (() => act(list[0].action))) : null;
    return `<button class="opt" ${ok ? `data-h="${h}"` : 'disabled'} title="${esc(ok ? list[0].text : label)}"><span class="oi">${inner}</span><span>${label}</span>${price !== '' ? `<span class="price">${price}</span>` : ''}</button>`;
  };
  const remaining = phaseBody(view.phase).remaining || 0;
  return `<div class="st-row">
      ${tile(arenaIcon({ size: rules().arena.squares_required_by_event_size.length - 1 }), 'Program', `${icon('coins')}${Number.isFinite(cheapest) ? `${cheapest}+` : '–'}`, buy, () => { showPrograms = true; render(); })}
      ${tile('<span class="plus">+</span>', 'Arena', `${icon('coins')}${rules().arena.expansion_cost}`, expand, expand.length > 1 ? () => toast('Click the + square on the board to pick a side') : null)}
      ${tile(icon('ticket'), 'Season ticket', `${icon('coins')}${inv.season_ticket_cost}`, ticket)}
      ${tile(icon('loge'), 'Loge', `${icon('coins')}${inv.emperor_loge_cost}`, loge)}
      ${special.length ? tile(icon('SpecialEvent'), 'Extra', '', special) : ''}
      ${medals.length ? tile(icon('medal'), 'Extra', `${icon('medal')}${inv.extra_investment_medal_cost}`, medals) : ''}
      ${tile(`<span class="plus">${CHECK}</span>`, 'Done', '', end)}</div>
    <div class="st-row"><span class="purse" title="Your coins">${icon('coins')}${view.players[me()].coins}</span>
      ${remaining ? `<span class="remaining" title="${remaining} investment${remaining === 1 ? '' : 's'} left">${Array.from({ length: remaining }, () => '<i></i>').join('')}</span>` : ''}</div>`;
}

// Acquiring: markets as big tiles; pick one and an opening bid, or bid on the live auction.
function acquiringStage(take) {
  const ph = phaseBody(view.phase);
  const auction = ph.auction;
  const starts = take('StartAuction');
  const bids = take('Bid');
  const passBid = take('PassBid');
  const passInit = take('PassInitiative');
  const marketTile = (m, i, clickable) => {
    const content = m.length ? m.map((a) => tok(a)).join('') : '';
    const cls = `mk ${m.length ? '' : 'empty'} ${(auction && auction.market === i) || ui.market === i ? 'on' : ''}`;
    return clickable
      ? `<button class="${cls}" title="Market ${i + 1}" data-h="${on(() => { ui.market = i; drawRing(); })}">${content}</button>`
      : `<div class="${cls}" title="Market ${i + 1}">${content}</div>`;
  };
  if (auction) {
    const amounts = bids.map((l) => body(l.action).amount).sort((a, b) => a - b);
    const bidders = auction.in_bidding.map((inb, i) => `<span class="who-chip ${inb ? '' : 'out'}" style="--pc:${pColor(i)}">${youOr(i)}</span>`).join('');
    return `<div class="st-row">${marketTile(view.markets[auction.market], auction.market, false)}</div>
      <div class="st-row"><span class="highbid" title="High bid">${icon('coins')}${auction.high_bid}</span><span class="who-chip" style="--pc:${pColor(auction.high_bidder)}">${youOr(auction.high_bidder)}</span></div>
      <div class="st-row">${bidders}</div>
      <div class="st-row">${amounts.length ? `${stepper('bid', amounts)}<button class="btn primary big" data-h="${on(() => act({ Bid: { amount: ui.bid.bid } }))}">Bid</button>` : ''}
        ${passBid.length ? `<button class="btn big" data-h="${on(() => act('PassBid'))}">Pass</button>` : ''}</div>`;
  }
  const openable = new Set(starts.map((l) => body(l.action).market));
  if (ui.market !== null && !openable.has(ui.market)) ui.market = null;
  if (ui.market === null && openable.size) ui.market = [...openable][0];
  const opening = starts.filter((l) => body(l.action).market === ui.market).map((l) => body(l.action).bid).sort((a, b) => a - b);
  return `<div class="st-row">${view.markets.map((m, i) => marketTile(m, i, openable.has(i))).join('')}</div>
    <div class="st-row">${opening.length ? `${stepper(`open${ui.market}`, opening)}<button class="btn primary big" data-h="${on(() => act({ StartAuction: { market: ui.market, bid: ui.bid[`open${ui.market}`] } }))}">Open auction</button>` : ''}
      ${passInit.length ? `<button class="btn big ghost" title="${esc(passInit[0].text)}" data-h="${on(() => act('PassInitiative'))}">Pass</button>` : ''}
      <span class="bag" title="Tokens left to draw">${view.bag_count}</span></div>`;
}
// A coin stepper over a sorted list of allowed values.
function stepper(key, values) {
  if (!values.includes(ui.bid[key])) ui.bid[key] = values[0];
  const i = values.indexOf(ui.bid[key]);
  const dec = on(() => { ui.bid[key] = values[Math.max(0, i - 1)]; drawRing(); });
  const inc = on(() => { ui.bid[key] = values[Math.min(values.length - 1, i + 1)]; drawRing(); });
  return `<span class="stepper"><button ${i === 0 ? 'disabled' : ''} data-h="${dec}">−</button><span>${icon('coins')}${ui.bid[key]}</span><button ${i === values.length - 1 ? 'disabled' : ''} data-h="${inc}">+</button></span>`;
}

// Trading: a table between two hands. Click tokens onto the table to give or
// ask for them; click them on the table to take them back.
function tradingStage(take) {
  const ph = phaseBody(view.phase);
  const offer = ph.offer;
  const mine = me();
  take('AcceptTrade', 'RejectTrade', 'EndTrading');
  const order = orderStrip(offer ? offer.from : (view.start_player + ph.turn) % n());
  if (ui.trade) return tradeBuilder(order);
  if (offer && offer.to === mine) {
    const t = offer.terms;
    const accept = legalOf('AcceptTrade');
    return `<div class="trow"><span class="who-chip" style="--pc:${pColor(offer.from)}">${pName(offer.from)}</span><span class="spacer"></span>${order}</div>
      <div class="hand-line dim" style="--pc:${pColor(offer.from)}">${handRow(view.players[offer.from].assets, { mark: t.give })}</div>
      <div class="deal big">${flow(offer.from, mine, t.give, t.give_coins)}${flow(mine, offer.from, t.receive, t.receive_coins)}</div>
      <div class="hand-line dim" style="--pc:${pColor(mine)}">${handRow(view.players[mine].assets, { mark: t.receive })}</div>
      <div class="trow"><span class="spacer"></span>
        <button class="round-btn no" title="Reject" data-h="${on(() => act('RejectTrade'))}">✕</button>
        <button class="round-btn counter" title="Counter-offer" ${view.can_counter ? `data-h="${on(() => {
          ui.trade = { to: offer.from, counter: true, was: offer, give: { ...t.receive }, giveCoins: t.receive_coins, receive: { ...t.give }, receiveCoins: t.give_coins };
          drawRing();
        })}"` : 'disabled'}>↺</button>
        <button class="round-btn yes" title="${accept.length ? 'Accept' : "You can't accept these terms"}" ${accept.length ? `data-h="${on(() => act('AcceptTrade'))}"` : 'disabled'}>✓</button></div>`;
  }
  if (view.can_propose && ensureTrade()) return tradeBuilder(order);
  return `<div class="trow"><span class="spacer"></span>${order}</div>`;
}
// A fresh offer to build whenever the human may propose one.
function ensureTrade() {
  if (!ui.trade && view.can_propose && phaseName(view.phase) === 'Trading') {
    const others = view.players.map((p) => p.id).filter((i) => i !== me());
    ui.trade = { to: others[0], counter: false, give: {}, giveCoins: 0, receive: {}, receiveCoins: 0 };
  }
  return ui.trade;
}
function orderStrip(now) {
  const seats = Array.from({ length: n() }, (_, k) => (view.start_player + k) % n());
  return `<span class="order">${seats.map((i) => `<span class="odot ${i === now ? 'now' : ''}" style="--pc:${pColor(i)}">${youOr(i)}</span>`).join('<span class="osep">›</span>')}</span>`;
}
const tradeable = (map) => Object.fromEntries(Object.entries(map).filter(([a]) => !UNTRADEABLE.has(a)));
// A hand: one tile per token in a row. `click(asset)` makes tiles buttons;
// `mark` outlines that many tiles of each type.
function handRow(map, opts = {}) {
  const list = entries(map);
  if (!list.length) return '<span class="muted">–</span>';
  return `<span class="hand">${list.map(([a, k]) => `<span class="hgrp">${Array.from({ length: k }, (_, j) => {
    const leaving = j < (opts.mark?.[a] || 0);
    return opts.click
      ? `<button class="tokbtn" title="${a}" data-h="${on(() => opts.click(a))}">${tok(a)}</button>`
      : `<span class="tokbtn ${leaving ? 'leaving' : ''}">${tok(a)}</span>`;
  }).join('')}</span>`).join('')}</span>`;
}
// One direction of a deal: [from] → tokens and coins → [to].
function flow(from, to, assets, coins, opts = {}) {
  const items = entries(assets).flatMap(([a, k]) => Array.from({ length: k }, () => (opts.click
    ? `<button class="tokbtn" title="Take back ${a}" data-h="${on(() => opts.click(a))}">${tok(a)}</button>`
    : tok(a)))).join('');
  const coinsPart = opts.coins
    ? `<span class="cstep"><button data-h="${on(() => opts.coins(-1))}">−</button><span>${icon('coins')}${coins}</span><button data-h="${on(() => opts.coins(1))}">+</button></span>`
    : coins ? `<span class="cpile">${icon('coins')}${coins}</span>` : '';
  return `<div class="flow" style="--a:${pColor(from)};--b:${pColor(to)}"><span class="who a">${youOr(from)}</span>
    <span class="tray">${items}${coinsPart}</span><span class="arrow">→</span><span class="who b">${youOr(to)}</span></div>`;
}
function tradeBuilder(order) {
  const tr = ui.trade;
  const mine = me();
  const them = tr.to;
  const add = (side, a) => { tr[side][a] = (tr[side][a] || 0) + 1; drawRing(); };
  const remove = (side, a) => { tr[side][a] -= 1; if (!tr[side][a]) delete tr[side][a]; drawRing(); };
  const coins = (key, max) => (d) => { tr[key] = Math.max(0, Math.min(max, tr[key] + d)); drawRing(); };
  const terms = () => ({ give: tr.give, give_coins: tr.giveCoins, receive: tr.receive, receive_coins: tr.receiveCoins });
  const partners = tr.counter ? '' : `<span class="row">${view.players.map((p) => p.id).filter((i) => i !== mine).map((i) =>
    `<button class="pdot ${them === i ? 'on' : ''}" style="--pc:${pColor(i)}" data-h="${on(() => { ui.trade = { ...tr, to: i, receive: {}, receiveCoins: 0 }; render(); })}">${pName(i)}</button>`).join('')}</span>`;
  const was = tr.counter ? `<span class="was" title="Their offer">${flow(tr.was.from, mine, tr.was.terms.give, tr.was.terms.give_coins)}${flow(mine, tr.was.from, tr.was.terms.receive, tr.was.terms.receive_coins)}</span>` : '';
  const empty = !sumMap(tr.give) && !sumMap(tr.receive) && !tr.giveCoins && !tr.receiveCoins;
  const send = on(() => act(tr.counter ? { CounterTrade: { terms: terms() } } : { ProposeTrade: { to: them, terms: terms() } }));
  const done = legalOf('EndTrading');
  return `<div class="trow">${tr.counter ? `<span class="who-chip" style="--pc:${pColor(them)}">↺ ${pName(them)}</span>` : partners}${was}<span class="spacer"></span>${order}</div>
    <div class="hand-line" style="--pc:${pColor(them)}">${handRow(subtract(tradeable(view.players[them].assets), tr.receive), { click: (a) => add('receive', a) })}</div>
    <div class="deal">${flow(mine, them, tr.give, tr.giveCoins, { click: (a) => remove('give', a), coins: coins('giveCoins', view.players[mine].coins) })}
      ${flow(them, mine, tr.receive, tr.receiveCoins, { click: (a) => remove('receive', a), coins: coins('receiveCoins', view.players[them].coins) })}</div>
    <div class="hand-line" style="--pc:${pColor(mine)}">${handRow(subtract(tradeable(view.players[mine].assets), tr.give), { click: (a) => add('give', a) })}</div>
    <div class="trow"><span class="spacer"></span>
      ${tr.counter ? `<button class="btn ghost" data-h="${on(() => { ui.trade = null; drawRing(); })}">Back</button>` : done.length ? `<button class="btn ghost" data-h="${on(() => act('EndTrading'))}">Done trading</button>` : ''}
      <button class="btn primary big" ${empty ? 'disabled' : `data-h="${send}"`}>${tr.counter ? 'Counter' : 'Offer'}</button></div>`;
}

// Producing: dice (Roman numerals) pick which moves the board shows; program
// cards produce once the dice are used.
function producingStage(take) {
  const ph = phaseBody(view.phase);
  const mine = me();
  take('MoveNoble', 'MoveNobleCombined');
  const medalMoves = take('MedalMoveNoble');
  const produce = take('ProduceEvent');
  const skip = take('SkipEvent');
  const die = activeDie();
  const usable = new Set(legalOf('MoveNoble').map((l) => body(l.action).die));
  const both = legalOf('MoveNobleCombined').length > 0;
  const dieBtn = (face, i) => {
    const used = ph.used[i];
    const ok = usable.has(i);
    const txt = roman(face);
    return `<button class="bigdie ${used ? 'used' : ''} ${!ui.medal && (die === i || die === 'both') && !used ? 'on' : ''}" ${ok ? `data-h="${on(() => { ui.die = i; ui.medal = false; ui.noble = null; drawRing(); })}"` : 'disabled'} title="${used ? 'Used' : `Move ${Array.isArray(face) ? face.join(', ') : face}`}"><span class="roman ${txt.length > 3 ? 'long' : ''}">${txt}</span></button>`;
  };
  const dice = ph.rolled ? `<div class="dicepair">${ph.dice.map((f, i) => (i ? `${ph.dice.length === 2 ? `<button class="link ${die === 'both' && !ui.medal ? 'on' : ''}" ${both ? `data-h="${on(() => { ui.die = 'both'; ui.medal = false; ui.noble = null; drawRing(); })}"` : 'disabled'} title="Both dice on one noble">+</button>` : ''}` : '') + dieBtn(f, i)).join('')}</div>` : '';
  // Medals to spend on the event (each adds spectators).
  const medalValues = [...new Set(produce.map((l) => body(l.action).medals))].sort((a, b) => a - b);
  if (!medalValues.includes(ui.medals)) ui.medals = medalValues[0] ?? 0;
  const cards = view.players[mine].programs.filter((p) => !p.produced).map((pr) => {
    const ev = event(pr.id);
    const options = produce.filter((l) => body(l.action).program === pr.id);
    const l = options.find((x) => body(x.action).medals === ui.medals) || options[0];
    const missing = missingFor(ev, view.players[mine].assets);
    const inner = `<span class="pt"><span class="pno sm">${ev.id}</span>${arenaIcon(ev)}<b>${esc(ev.name)}</b></span>
      ${falloff(ev, missing <= ev.penalty_scores.length ? missing : null)}${needRow(ev, view.players[mine].assets, 'sm')}
      ${l ? `<span class="pscore">${icon('spectators')}${spectatorsIn(l.text)}</span>` : ''}`;
    return l ? `<button class="pcard" title="${esc(l.text)}" data-h="${on(() => act(l.action))}">${inner}</button>` : `<div class="pcard off">${inner}</div>`;
  }).join('');
  const medalPick = medalValues.length > 1 ? `<span class="medalpick">${icon('medal')}${stepperPlain(medalValues)}</span>` : '';
  const tools = [
    medalMoves.length ? `<button class="btn small ${ui.medal ? 'primary' : ''}" title="Spend a medal to move a noble" data-h="${on(() => { ui.medal = !ui.medal; ui.noble = null; drawRing(); })}">${icon('medal')} move</button>` : '',
    medalPick,
    skip.length ? `<button class="btn small ghost" data-h="${on(() => act('SkipEvent'))}">Skip</button>` : '',
  ].join('');
  return `<div class="prod">${dice}<div class="pcards">${cards || ''}</div></div>${tools.trim() ? `<div class="st-foot">${tools}</div>` : ''}`;
}
function stepperPlain(values) {
  const i = values.indexOf(ui.medals);
  const dec = on(() => { ui.medals = values[Math.max(0, i - 1)]; drawRing(); });
  const inc = on(() => { ui.medals = values[Math.min(values.length - 1, i + 1)]; drawRing(); });
  return `<span class="stepper"><button ${i === 0 ? 'disabled' : ''} data-h="${dec}">−</button><span>${ui.medals}</span><button ${i === values.length - 1 ? 'disabled' : ''} data-h="${inc}">+</button></span>`;
}

// Closing: this round's events as a podium, then the discard or the donation.
function closingStage(take) {
  const ph = phaseBody(view.phase);
  const discards = take('DiscardAsset');
  const requests = take('RequestAsset');
  const results = view.players.map((p) => [p.id, p.last_event && p.last_event.round === view.round ? p.last_event.spectators : 0])
    .sort((a, b) => b[1] - a[1]);
  const top = Math.max(1, results[0][1]);
  const podium = `<div class="podium">${results.map(([i, s]) => `<div class="step" style="--pc:${pColor(i)};--h:${Math.max(14, (100 * s) / top)}%">
    <span class="who">${youOr(i)}</span><span class="bar">${ph.podium_winner === i ? icon('podium') : ''}<b>${s || '–'}</b></span></div>`).join('')}</div>`;
  const pick = (list) => `<span class="toks bigtoks">${list.map((l) => `<button class="tokbtn" title="${esc(l.text)}" data-h="${on(() => act(l.action))}">${tok(body(l.action).asset)}</button>`).join('')}</span>`;
  return `${podium}
    ${discards.length ? `<div class="st-row"><span class="muted">Discard one</span>${pick(discards)}</div>` : ''}
    ${requests.length ? `<div class="st-row"><span class="who-chip" style="--pc:${pColor(ph.podium_winner)}">${pName(ph.podium_winner)}</span><span class="muted">gives you one</span>${pick(requests)}</div>` : ''}`;
}

function gameOverInner() {
  const rows = view.ranking.map((i, k) => `<div class="rank"><b>${k + 1}.</b><span class="dot" style="--pc:${pColor(i)}"></span>${youOr(i)}
    <span class="muted">${SEAT_LABEL[view.seats[i]]}</span><span class="score">${icon('spectators')}${view.players[i].best_score}</span></div>`).join('');
  return `<div class="card" style="min-width:300px"><h3>Final ranking</h3>${rows}<div class="btns" style="margin-top:12px"><button class="btn primary" data-h="${on(newGame)}">New game</button></div></div>`;
}

// ---------- bench ----------
function extras(p) {
  return `${p.loge ? `<span title="Emperor's loge">${icon('loge')}</span>` : ''}${repeatIcon('ticket', p.season_tickets, 'season tickets')}${repeatIcon('medal', p.medals, 'medals')}${repeatIcon('podium', p.podiums, 'podiums')}`;
}
// A held program. Unproduced ones show the tokens it needs (faded: not held)
// and its spectator fall-off with the score the owner would get now
// highlighted; the arena outline turns red when the owner's arena is too small.
function programRow(pr, owner) {
  const ev = event(pr.id);
  const p = view.players[owner];
  if (pr.produced) {
    return `<div class="prow done" title="${esc(ev.name)} (produced)"><span class="pno sm">${ev.id}</span>${arenaIcon(ev)}<b>${esc(ev.name)}</b>${CHECK}</div>`;
  }
  const missing = missingFor(ev, p.assets);
  const cur = missing <= ev.penalty_scores.length ? missing : null;
  const small = p.arena.len < rules().arena.squares_required_by_event_size[ev.size];
  return `<div class="prow detail" title="${esc(ev.name)}">
    <div class="pr1"><span class="pno sm">${ev.id}</span><span class="${small ? 'too-small' : ''}" title="${small ? 'Arena too small' : ''}">${arenaIcon(ev)}</span><b>${esc(ev.name)}</b>${falloff(ev, cur, true)}</div>
    ${needRow(ev, p.assets, 'xs')}</div>`;
}
function anytimeButtons() {
  if (!view.human_to_act) return '';
  const ex = legalOf('ExchangeMedalForCoins', 'ExchangeSpecialEventForMedal');
  return ex.length ? `<span class="anytime">${ex.map((l) => `<button class="btn small" title="${esc(l.text)}" data-h="${on(() => act(l.action))}">${
    kind(l.action) === 'ExchangeMedalForCoins' ? `${icon('medal')}→${icon('coins')}` : `${icon('SpecialEvent')}→${icon('medal')}`}</button>`).join('')}</span>` : '';
}
function mySeat(i) {
  const p = view.players[i];
  return `<div class="seat me ${view.current_player === i ? 'turn' : ''}" style="--pc:${pColor(i)}">
    <div class="col-a"><div class="sh"><span class="dot"></span><b>${isHuman(i) ? 'You' : pName(i)}</b><span class="muted kind">${pName(i)}${view.start_player === i ? ' · starts' : ''}</span><span class="spacer"></span><span class="best" title="Best event">${icon('spectators')}${p.best_score}</span></div>
      <div class="sm"><span class="coins" title="Coins">${icon('coins')}${p.coins}</span><span class="ex">${extras(p)}</span><span>arena ${p.arena.len}</span><span class="spacer"></span>${anytimeButtons()}</div>
      ${seatToks(i, 'sm')}</div>
    <div class="col-b">${[...p.programs].sort((a, b) => a.produced - b.produced).map((pr) => programRow(pr, i)).join('')}</div></div>`;
}
function oppSeat(i) {
  const p = view.players[i];
  const ph = phaseName(view.phase);
  const offer = ph === 'Trading' ? phaseBody(view.phase).offer : null;
  const partner = (ui.trade && ui.trade.to === i) || (offer && (offer.from === i || offer.to === i));
  const pickable = ui.trade && !ui.trade.counter;
  const pills = p.programs.map((pr) => { const ev = event(pr.id); return `<span class="ppill ${pr.produced ? 'done' : ''}" title="${ev.id} · ${esc(ev.name)}"><span class="pno xs">${ev.id}</span>${pr.produced ? CHECK : arenaIcon(ev)}</span>`; }).join('');
  return `<div class="seat opp ${view.current_player === i ? 'turn' : ''} ${partner ? 'partner' : ''}" style="--pc:${pColor(i)}" ${pickable ? `data-h="${on(() => { ui.trade = { ...ui.trade, to: i, receive: {}, receiveCoins: 0 }; render(); })}"` : ''}>
    <div class="sh"><span class="dot"></span><b>${pName(i)}</b><span class="muted kind">${SEAT_LABEL[view.seats[i]]}${view.start_player === i ? ' · starts' : ''}</span><span class="spacer"></span><span class="best" title="Best event">${icon('spectators')}${p.best_score}</span></div>
    <div class="sm"><span class="coins" title="Coins">${icon('coins')}${p.coins}</span><span class="ex">${extras(p)}</span><span class="spacer"></span><span class="ppills">${pills}</span></div>
    ${seatToks(i, 'xs')}
    <div class="pop">${[...p.programs].sort((a, b) => a.produced - b.produced).map((pr) => programRow(pr, i)).join('')}</div></div>`;
}
function bench() {
  const mine = me();
  const others = view.players.map((p) => p.id).filter((i) => i !== mine);
  return `<div class="bench" style="--o:${others.length}">${mySeat(mine)}${others.map(oppSeat).join('')}</div>`;
}

// ---------- programs for sale ----------
function programsModal() {
  const mine = view.players[me()];
  const buyable = new Map(legalOf('BuyProgram').map((l) => [body(l.action).program, l]));
  const rows = view.programs_for_sale.map(event).map((ev) => {
    const l = buyable.get(ev.id);
    const inner = `<span class="pno">${ev.id}</span><span class="cost">${icon('coins')}${ev.cost}</span>
      <span class="t">${esc(ev.name)}${arenaIcon(ev)}</span>${falloff(ev)}
      <span class="hold">hold <b>${matched(ev, mine.assets)}</b>/${ev.requirements.length}</span>${needRow(ev, mine.assets, 'sm')}`;
    return l
      ? `<button class="plist-row" title="${esc(l.text)}" data-h="${on(() => act(l.action))}">${inner}</button>`
      : `<div class="plist-row ${ev.cost > mine.coins ? 'dim' : ''}">${inner}</div>`;
  }).join('');
  return `<div class="modal" data-h="${on(() => { showPrograms = false; render(); })}"><div class="panel" data-stop>
    <div class="mhead"><h2>Programs for sale</h2><span class="muted">${view.programs_for_sale.length} left</span><span class="spacer"></span>
      ${buyable.size ? '<span class="hint">Click one to buy it</span>' : ''}<button class="btn small ghost" data-h="${on(() => { showPrograms = false; render(); })}">Close <kbd>Esc</kbd></button></div>
    <div class="plist-head"><span>No.</span><span>Cost</span><span>Program</span><span class="falloff head">${[0, 1, 2, 3, 4, 5, 6].map((k) => `<span>${k}</span>`).join('')}</span><span></span></div>
    <div class="plist-head" style="padding-top:0;border:0;margin-top:-4px"><span></span><span></span><span></span><span style="text-transform:none;letter-spacing:0">${icon('spectators')} spectators by tokens missing</span><span></span></div>
    <div class="plist-body">${rows || '<div class="hint" style="padding:12px">Sold out</div>'}</div>
    <div class="plist-foot"><span>Faded tokens: ones you don't hold</span><span>Programs can be bought while investing</span></div></div></div>`;
}

// ---------- log ----------
function logDrawer() {
  const lines = view.log.map((e) => `<div class="e ${e.kind} ${e.n >= logSeen && e.kind === 'event' ? 'new' : ''}">${esc(e.text)}</div>`).join('');
  return `<div class="drawer"><div class="row"><b>Log</b><span class="spacer"></span><span class="hint">seed ${view.seed}</span><button class="btn small ghost" data-h="${on(toggleLog)}">Close <kbd>L</kbd></button></div><div class="log" id="log">${lines}</div></div>`;
}

function newGame() {
  view = null;
  showLog = false;
  showPrograms = false;
  history.replaceState(null, '', location.pathname);
  render();
}

// ---------- top level ----------
function render() {
  handlers = [];
  if (ringObserver) { ringObserver.disconnect(); ringObserver = null; }
  if (!view) {
    document.body.classList.remove('playing');
    $app.innerHTML = renderSetup();
    return;
  }
  document.body.classList.add('playing');
  // Seat clicks only pick a trade partner while building a fresh offer.
  if (ui.trade && (phaseName(view.phase) !== 'Trading' || !view.human_to_act)) ui.trade = null;
  ensureTrade();
  $app.innerHTML = `<div class="app">${topBar()}<div class="ring-area" id="ring"></div>${bench()}</div>
    ${showPrograms ? programsModal() : ''}${showLog ? logDrawer() : ''}`;
  const ring = document.getElementById('ring');
  let last = '';
  ringObserver = new ResizeObserver(() => {
    const r = ring.getBoundingClientRect();
    const key = `${Math.round(r.width)}x${Math.round(r.height)}`;
    if (key !== last) { last = key; drawRing(); }
  });
  ringObserver.observe(ring);
  drawRing();
  const log = document.getElementById('log');
  if (log) log.scrollTop = log.scrollHeight;
}

document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-h], [data-stop]');
  if (!el || el.hasAttribute('data-stop')) return;
  const fn = handlers[Number(el.dataset.h)];
  if (fn) fn();
});

document.addEventListener('change', (e) => {
  const el = e.target.closest('select.seat-kind');
  if (!el) return;
  setup.seats[Number(el.dataset.seat)] = el.value;
  render();
});

document.addEventListener('keydown', (e) => {
  if (!view || e.target.matches('input, select, textarea') || e.ctrlKey || e.metaKey || e.altKey) return;
  const k = e.key.toLowerCase();
  if (k === 'p') { showPrograms = !showPrograms; render(); }
  else if (k === 'l') toggleLog();
  else if (e.key === 'Escape' && (showPrograms || showLog || ui.noble !== null || ui.medal)) {
    if (showLog) logSeen = view.log.length;
    showPrograms = false;
    showLog = false;
    ui.noble = null;
    ui.medal = false;
    render();
  }
});

(async function boot() {
  config = await api('/api/config');
  const id = location.hash.slice(1);
  if (id) {
    try { view = await api(`/api/games/${id}`); logSeen = view.log.length; } catch { view = null; }
  }
  render();
})();
