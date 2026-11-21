// Shared sample state and render helpers for the layout mockups.
// The state is a hand-made snapshot shaped like the server's View (round 3,
// producing phase, P1 to act with two dice). Nothing here talks to the server.
'use strict';

// ---------- presentation theme (copied from static/app.js) ----------
const TOKENS = {
  Gladiator: { bg: '#fcd4d4', fg: '#b3343a' }, Ship: { bg: '#cdeef0', fg: '#1b7f88' },
  Musician: { bg: '#dcf5d2', fg: '#4c9a2a' }, Comedian: { bg: '#fdeca0', fg: '#8f6c00' },
  Priest: { bg: '#ffffff', fg: '#5b616e' }, Horse: { bg: '#ead9c6', fg: '#85552a' },
  Lion: { bg: '#ffd8b5', fg: '#c25a0c' }, Chariot: { bg: '#e6dafc', fg: '#6a45c2' },
  Cage: { bg: '#eceef2', fg: '#7a808c' }, Torch: { bg: '#9aa0aa', fg: '#2f333a' },
  Scenery: { bg: '#cfe2fb', fg: '#2c66b3' }, Decoration: { bg: '#dde6bd', fg: '#5c6a1c' },
  Joker: { bg: '#ffffff', fg: '#3b3f48' }, Emperor: { bg: '#fff1b8', fg: '#94700a' },
  SpecialEvent: { bg: '#efe6ff', fg: '#7a4fc0' },
};
const NOBLES = {
  Emperor: { bg: '#ffd6d2', fg: '#a8241a' }, Consul: { bg: '#d6e6ff', fg: '#1f57b8' },
  Senator: { bg: '#ffeaa0', fg: '#7a5a00' },
};

// ---------- rules data (from data/*.yml) ----------
const RULES = {
  board: 36, rounds: 5, rests: [0, 6, 12, 19, 26, 30], arenaBySize: [2, 3, 4], prevBonus: 5, starThreshold: 3,
};
const EVENTS = [{"id":1,"name":"Tribute to Munus","cost":0,"base_score":7,"size":0,"requirements":["Gladiator","Gladiator","Decoration"],"penalty_scores":[5,3]},{"id":2,"name":"Amphitryon","cost":0,"base_score":7,"size":0,"requirements":["Comedian","Comedian","Decoration"],"penalty_scores":[5,3]},{"id":3,"name":"Ode to Aurora","cost":0,"base_score":7,"size":0,"requirements":["Musician","Musician","Scenery"],"penalty_scores":[5,3]},{"id":4,"name":"Race of Janus","cost":0,"base_score":9,"size":0,"requirements":["Gladiator","Gladiator","Horse","Horse"],"penalty_scores":[7,5,3]},{"id":5,"name":"Naumachia","cost":0,"base_score":9,"size":0,"requirements":["Gladiator","Gladiator","Ship","Ship"],"penalty_scores":[7,5,3]},{"id":6,"name":"Venatio","cost":0,"base_score":11,"size":0,"requirements":["Horse","Horse","Lion","Lion","Cage"],"penalty_scores":[9,7,5,2]},{"id":7,"name":"The Last of the Samnites","cost":0,"base_score":11,"size":0,"requirements":["Gladiator","Gladiator","Gladiator","Scenery","Decoration"],"penalty_scores":[9,7,5,2]},{"id":8,"name":"Drama Ultima","cost":0,"base_score":12,"size":0,"requirements":["Comedian","Comedian","Comedian","Torch","Scenery","Scenery"],"penalty_scores":[10,8,5,2]},{"id":9,"name":"Anthem to Cupidon","cost":0,"base_score":12,"size":0,"requirements":["Musician","Musician","Musician","Comedian","Scenery","Decoration"],"penalty_scores":[10,8,5,2]},{"id":10,"name":"Cavalry of Spartacus","cost":0,"base_score":12,"size":0,"requirements":["Gladiator","Gladiator","Gladiator","Horse","Horse","Horse"],"penalty_scores":[10,8,5,2]},{"id":11,"name":"Amphitrite's Anger","cost":13,"base_score":17,"size":1,"requirements":["Gladiator","Gladiator","Gladiator","Ship","Ship","Ship","Decoration","Decoration"],"penalty_scores":[14,11,8,5,2]},{"id":12,"name":"Cave Leonem Cruciatum","cost":13,"base_score":17,"size":1,"requirements":["Gladiator","Gladiator","Gladiator","Lion","Lion","Lion","Cage","Cage"],"penalty_scores":[14,11,8,5,2]},{"id":13,"name":"Retiarius vs. Secutor","cost":14,"base_score":18,"size":1,"requirements":["Gladiator","Gladiator","Horse","Horse","Chariot","Chariot","Scenery","Decoration"],"penalty_scores":[15,12,9,6,3]},{"id":14,"name":"Satyricon of Momus","cost":14,"base_score":18,"size":1,"requirements":["Comedian","Comedian","Priest","Priest","Torch","Torch","Scenery","Decoration"],"penalty_scores":[15,12,9,6,3]},{"id":15,"name":"Lyres of Mercury","cost":17,"base_score":20,"size":1,"requirements":["Musician","Musician","Musician","Priest","Chariot","Torch","Torch","Decoration","Decoration"],"penalty_scores":[17,14,11,7,3]},{"id":16,"name":"The Ponies of Epona","cost":20,"base_score":23,"size":1,"requirements":["Gladiator","Gladiator","Gladiator","Priest","Horse","Horse","Horse","Scenery","Scenery"],"penalty_scores":[20,16,12,8,4]},{"id":17,"name":"Bacchus and the Nymphs","cost":21,"base_score":25,"size":1,"requirements":["Musician","Musician","Priest","Priest","Priest","Torch","Torch","Torch","Scenery"],"penalty_scores":[22,18,14,10,5]},{"id":18,"name":"Mare Spielbergus","cost":22,"base_score":27,"size":1,"requirements":["Gladiator","Gladiator","Gladiator","Ship","Ship","Ship","Cage","Cage","Scenery"],"penalty_scores":[22,18,14,10,5]},{"id":19,"name":"Lions' Procession","cost":24,"base_score":30,"size":1,"requirements":["Musician","Musician","Comedian","Lion","Lion","Lion","Cage","Cage","Decoration"],"penalty_scores":[26,22,18,14,9,4]},{"id":20,"name":"Circus Maximus","cost":25,"base_score":32,"size":1,"requirements":["Gladiator","Gladiator","Gladiator","Horse","Horse","Horse","Chariot","Chariot","Chariot","Decoration"],"penalty_scores":[28,24,20,15,10,5]},{"id":21,"name":"The Dialogues of Pluto","cost":25,"base_score":33,"size":2,"requirements":["Comedian","Comedian","Chariot","Chariot","Cage","Cage","Cage","Torch","Scenery","Scenery"],"penalty_scores":[29,25,20,15,10,5]},{"id":22,"name":"And Tellus Created the Earth","cost":26,"base_score":35,"size":2,"requirements":["Musician","Comedian","Comedian","Priest","Priest","Priest","Chariot","Torch","Torch","Decoration","Decoration"],"penalty_scores":[30,25,20,15,10,5]},{"id":23,"name":"Safari Optimus Maximus","cost":27,"base_score":36,"size":2,"requirements":["Gladiator","Gladiator","Gladiator","Gladiator","Lion","Lion","Lion","Cage","Cage","Scenery","Scenery"],"penalty_scores":[31,26,21,16,11,5]},{"id":24,"name":"The Galleys of Apollo","cost":28,"base_score":38,"size":2,"requirements":["Gladiator","Gladiator","Gladiator","Gladiator","Ship","Ship","Ship","Ship","Comedian","Torch","Decoration"],"penalty_scores":[33,28,23,18,12,6]},{"id":25,"name":"Legendary Lion Hunters","cost":29,"base_score":42,"size":2,"requirements":["Gladiator","Gladiator","Gladiator","Comedian","Lion","Lion","Lion","Cage","Cage","Torch","Scenery","Scenery","Decoration"],"penalty_scores":[37,32,27,21,15,8]},{"id":26,"name":"Venus' Equestrian Cavalcade","cost":32,"base_score":44,"size":2,"requirements":["Musician","Musician","Musician","Comedian","Priest","Horse","Horse","Horse","Chariot","Chariot","Chariot","Torch","Scenery"],"penalty_scores":[39,33,27,21,15,8]},{"id":27,"name":"Neptune's Fury","cost":35,"base_score":45,"size":2,"requirements":["Gladiator","Gladiator","Gladiator","Ship","Ship","Ship","Ship","Comedian","Comedian","Priest","Torch","Torch","Decoration"],"penalty_scores":[40,34,28,22,15,8]},{"id":28,"name":"Isis' Revealed Secrets","cost":35,"base_score":45,"size":2,"requirements":["Musician","Musician","Musician","Comedian","Comedian","Comedian","Priest","Priest","Torch","Torch","Scenery","Scenery","Decoration"],"penalty_scores":[40,34,28,22,15,8]},{"id":29,"name":"Great Prophecies of Jupiter","cost":37,"base_score":47,"size":2,"requirements":["Gladiator","Musician","Comedian","Priest","Priest","Priest","Horse","Torch","Torch","Torch","Scenery","Scenery","Decoration","Decoration"],"penalty_scores":[42,37,31,25,18,8]},{"id":30,"name":"Caesar's Triumphal March","cost":40,"base_score":50,"size":2,"requirements":["Gladiator","Gladiator","Musician","Musician","Comedian","Comedian","Priest","Horse","Horse","Horse","Chariot","Chariot","Chariot","Torch","Torch","Decoration"],"penalty_scores":[45,40,34,28,20,10]}];
const event = (id) => EVENTS.find((e) => e.id === id);

// ---------- the sample game ----------
const STATE = {
  round: 3, phase: 'Producing', me: 0, current: 0, start: 2, seed: 41207,
  players: [
    { seat: 'Human', coins: 23, arena: { start: 3, len: 3 }, loge: true, tickets: 1, medals: 2, podiums: 0, best: 18,
      assets: { Gladiator: 4, Horse: 3, Ship: 1, Lion: 1, Scenery: 1, Decoration: 2, Joker: 1 },
      programs: [{ id: 4, produced: true }, { id: 10 }, { id: 13 }] },
    { seat: 'Oracle bot', coins: 31, arena: { start: 14, len: 2 }, loge: false, tickets: 0, medals: 1, podiums: 0, best: 12,
      assets: { Comedian: 4, Musician: 3, Priest: 2, Torch: 1, Scenery: 2 },
      programs: [{ id: 2, produced: true }, { id: 9 }, { id: 14 }] },
    { seat: 'Maestro bot', coins: 17, arena: { start: 23, len: 3 }, loge: false, tickets: 2, medals: 0, podiums: 1, best: 23,
      assets: { Musician: 2, Priest: 3, Torch: 2, Chariot: 1, Decoration: 1 },
      programs: [{ id: 3, produced: true }, { id: 8, produced: true }, { id: 17 }] },
    { seat: 'Greedy bot', coins: 40, arena: { start: 32, len: 2 }, loge: false, tickets: 0, medals: 0, podiums: 0, best: 9,
      assets: { Gladiator: 3, Ship: 3, Lion: 2, Cage: 2 },
      programs: [{ id: 5, produced: true }, { id: 12 }] },
  ],
  nobles: [
    { kind: 'Emperor', position: 4 }, { kind: 'Senator', position: 9 }, { kind: 'Consul', position: 15 },
    { kind: 'Senator', position: 19 }, { kind: 'Consul', position: 28 }, { kind: 'Senator', position: 33 },
  ],
  dice: [[1, 2, 3], 5], used: [false, false],
  markets: [['Gladiator', 'Horse', 'Torch'], ['Lion', 'Cage', 'Scenery'], [], ['Musician', 'Comedian', 'Joker'], ['Ship', 'Priest', 'Decoration']],
  bag: 61, supply: { medals: 12, tickets: 7 },
  stars: { Gladiator: 0, Horse: 0, Musician: 1, Comedian: 1, Priest: 2, Ship: 3, Lion: null },
  forSale: [11, 15, 16, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30],
  log: [
    { k: 'round', t: 'Round 3' },
    { k: 'phase', t: 'Investing' },
    { k: 'event', t: 'P3 bought Bacchus and the Nymphs for 21' },
    { k: 'event', t: 'P4 bought Cave Leonem Cruciatum for 13' },
    { k: 'event', t: 'P1 bought Retiarius vs. Secutor for 14' },
    { k: 'event', t: 'P2 bought Satyricon of Momus for 14' },
    { k: 'phase', t: 'Acquiring' },
    { k: 'event', t: 'P3 opened market 3 at 8' },
    { k: 'event', t: 'P1 bid 9 · P4 bid 11 · P1 passed' },
    { k: 'event', t: 'P4 won market 3 for 11: Lion, Cage, Gladiator' },
    { k: 'event', t: 'P1 won market 2 for 8: Horse, Horse, Decoration' },
    { k: 'phase', t: 'Trading' },
    { k: 'event', t: 'P2 traded Musician to P3 for 3 coins' },
    { k: 'phase', t: 'Producing' },
    { k: 'event', t: 'P1 rolled 1–3 and 5', n: true },
  ],
};

// ---------- helpers ----------
const pName = (i) => `P${i + 1}`;
const pColor = (i) => `var(--p${i})`;
const ORDER = Object.keys(TOKENS);
const entries = (map) => ORDER.filter((a) => map[a] > 0).map((a) => [a, map[a]]);
const count = (list) => list.reduce((m, a) => ((m[a] = (m[a] || 0) + 1), m), {});
const icon = (name, cls = '') => `<svg class="ic ${cls}" viewBox="0 0 512 512" aria-hidden="true"><path fill="currentColor" d="${ICON_PATHS[name]}"/></svg>`;
const CHECK = '<svg class="ic" viewBox="0 0 512 512" aria-hidden="true"><path d="M96 272l112 112L416 144" fill="none" stroke="currentColor" stroke-width="56" stroke-linecap="round" stroke-linejoin="round"/></svg>';
const faceLabel = (d) => (Array.isArray(d) ? `${Math.min(...d)}–${Math.max(...d)}` : d);

function tok(asset, opts = {}) {
  const t = TOKENS[asset];
  const cls = ['tok', opts.size || '', asset === 'Joker' ? 'joker' : '', t.bg === '#ffffff' ? 'white' : '', opts.miss ? 'miss' : ''].join(' ');
  const n = opts.n > 1 ? ` data-n="${opts.n}"` : '';
  return `<span class="${cls}" style="--c:${t.bg};color:${t.fg}" title="${asset}"${n}>${icon(asset)}</span>`;
}
// One tile per token, one column per type (like the current GUI).
function toks(map, opts = {}) {
  const list = entries(map);
  if (!list.length) return '<span class="muted">none</span>';
  return `<span class="toks">${list.map(([a, n]) => `<span class="tcol">${Array.from({ length: n }, () => tok(a, opts)).join('')}</span>`).join('')}</span>`;
}
// One tile per type with a count badge: the compact form.
function stacks(map, opts = {}) {
  return `<span class="toks">${entries(map).map(([a, n]) => tok(a, { ...opts, n })).join('')}</span>`;
}
// An event's needs as a row; tokens the holder lacks are faded.
function needRow(ev, have = null, opts = {}) {
  const left = have ? { ...have } : null;
  return `<span class="mrow need">${ev.requirements.map((a) => {
    const ok = !left || left[a] > 0;
    if (left && ok) left[a] -= 1;
    return tok(a, { ...opts, miss: !ok });
  }).join('')}</span>`;
}
function matched(ev, have) {
  const left = { ...have };
  let n = 0;
  for (const a of ev.requirements) if (left[a] > 0) { left[a] -= 1; n += 1; }
  return n;
}
function ladder(ev, short = false) {
  const steps = (short ? ev.penalty_scores.slice(0, 2) : ev.penalty_scores)
    .map((s) => `<span class="sep">›</span><span class="lstep">${s}</span>`).join('');
  return `<span class="ladder">${icon('spectators')}<b>${ev.base_score}</b>${steps}${short && ev.penalty_scores.length > 2 ? '<span class="sep">…</span>' : ''}</span>`;
}
function arenaIcon(ev) {
  const n = RULES.arenaBySize[ev.size];
  let x = 8;
  let d = 'M8 2A6 6 0 0 0 8 14';
  for (let i = 2; i < n; i++) { x += 3; d += `M${x} 2h6M${x} 14h6`; x += 6; }
  x += 3;
  d += `M${x} 2A6 6 0 0 1 ${x} 14`;
  return `<svg class="arena-ic" viewBox="0 0 ${x + 8} 16" style="width:${(x + 8) * 1.05}px"><title>Needs an arena of ${n}</title><path d="${d}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>`;
}
const bonusBadge = () => `<span class="bonus">${icon('spectators')} +${RULES.prevBonus}</span>`;
const starsOf = (i) => Object.entries(STATE.stars).filter(([, h]) => h === i).map(([a]) => a);

// ---------- board ----------
function squarePos(i, n) {
  const s = n - 1;
  if (i < s) return [0, i];
  if (i < 2 * s) return [i - s, s];
  if (i < 3 * s) return [s, s - (i - 2 * s)];
  return [s - (i - 3 * s), 0];
}
function arenaOwner(sq) {
  const N = RULES.board;
  for (let i = 0; i < STATE.players.length; i++) {
    const a = STATE.players[i].arena;
    if ((sq - a.start + N) % N < a.len) return i;
  }
  return null;
}
// Squares the selected die (the 5) can send a noble to; one is highlighted.
const TARGETS = new Set(STATE.nobles.map((nb) => (nb.position + 5) % RULES.board));
function board(center, opts = {}) {
  const N = RULES.board;
  const n = N / 4 + 1;
  const rest = new Set(RULES.rests);
  const at = {};
  STATE.nobles.forEach((nb) => (at[nb.position] ||= []).push(nb));
  let cells = '';
  for (let sq = 0; sq < N; sq++) {
    const [r, c] = squarePos(sq, n);
    const owner = arenaOwner(sq);
    const target = opts.targets !== false && TARGETS.has(sq);
    const cls = ['sq', rest.has(sq) ? 'rest' : '', owner !== null ? 'arena' : '', target ? 'target' : '', target && sq === 9 ? 'hot' : ''].join(' ');
    const style = `grid-row:${r + 1};grid-column:${c + 1};${owner !== null ? `--pc:${pColor(owner)}` : ''}`;
    const nobles = (at[sq] || []).map((nb) => `<span class="noble" style="--nc:${NOBLES[nb.kind].bg};color:${NOBLES[nb.kind].fg}" title="${nb.kind}">${icon(nb.kind)}</span>`).join('');
    cells += `<div class="${cls}" style="${style}"><span class="num">${sq}</span>${nobles}</div>`;
  }
  const mid = center === null ? '' : `<div class="board-center" style="grid-row:2/${n};grid-column:2/${n}">${center}</div>`;
  return `<div class="board" style="--n:${n}">${cells}${mid}</div>`;
}
function marketsHtml(opts = {}) {
  return `<div class="markets">${STATE.markets.map((m, i) => `<div class="market ${m.length ? '' : 'empty'}">
    <span class="label">Market ${i + 1}</span>${m.length ? `<span class="mrow">${m.map((a) => tok(a, opts)).join('')}</span>` : '<span class="muted" style="font-size:12px">empty</span>'}</div>`).join('')}</div>`;
}
function supplyHtml() {
  const unclaimed = Object.entries(STATE.stars).filter(([, h]) => h === null).map(([a]) => a);
  return `<div class="row"><span class="stat">${icon('medal')} ${STATE.supply.medals} medals</span>
    <span class="stat">${icon('ticket')} ${STATE.supply.tickets} tickets</span>
    <span class="stat">${STATE.bag} in bag</span>
    ${unclaimed.length ? `<span class="hint">Unclaimed star</span>${unclaimed.map((a) => tok(a, { size: 'sm' })).join('')}` : ''}</div>`;
}
function diceHtml(sel = 1) {
  return `<span class="dice">${STATE.dice.map((d, i) => `<button class="die ${i === sel ? 'on' : ''}">${faceLabel(d)}</button>`).join('')}<button class="die">Both</button></span>`;
}

// ---------- action panel content (producing, P1 holding the 5) ----------
function moveRows() {
  const me = STATE.me;
  return STATE.nobles.map((nb) => {
    const to = (nb.position + 5) % RULES.board;
    const owner = arenaOwner(to);
    const tag = owner === me ? '<b>your arena</b>' : owner !== null ? `${pName(owner)}'s arena` : RULES.rests.includes(to) ? `${icon('medal')} medal` : '';
    return { nb, to, tag };
  });
}
function actionsHtml(opts = {}) {
  const rows = moveRows().map(({ nb, to, tag }) => `<div class="move-row">
      <span class="who"><span style="color:${NOBLES[nb.kind].fg}">${icon(nb.kind)}</span> ${nb.kind} <span class="muted">${nb.position}</span></span>
      <button class="btn small">→ ${to}</button><span class="hint">${tag}</span></div>`).join('');
  const produce = [[10, 12], [13, 9]].map(([id, sp]) => `<button class="btn produce"><span>Produce <b>${event(id).name}</b></span><span class="spacer"></span>${icon('spectators')} <b>${sp}</b></button>`).join('');
  return `
    <div class="act-sec"><div class="row between"><b>Move a noble</b>${diceHtml()}</div>
      <span class="hint">Die <b>5</b> selected. Each die moves a different noble.</span>
      <div class="moves">${rows}</div></div>
    ${opts.noProduce ? '' : `<div class="act-sec"><b>Produce an event</b><span class="hint">Use both dice first.</span><div class="produce-list">${produce}</div></div>`}
    <div class="act-sec"><div class="btns"><button class="btn small">${icon('medal')} Spend a medal to move…</button><button class="btn small">Exchange medal → 6 coins</button></div></div>`;
}

// ---------- player summary pieces ----------
function playerStats(p) {
  const out = [`<span title="coins">${icon('coins')} <b>${p.coins}</b></span>`, `<span>arena <b>${p.arena.len}</b></span>`];
  if (p.loge) out.push(`<span>${icon('loge')} loge</span>`);
  if (p.tickets) out.push(`<span>${icon('ticket')} <b>${p.tickets}</b></span>`);
  if (p.medals) out.push(`<span>${icon('medal')} <b>${p.medals}</b></span>`);
  if (p.podiums) out.push(`<span>${icon('podium')} <b>${p.podiums}</b></span>`);
  return out.join('');
}
function programLine(pr, have, opts = {}) {
  const ev = event(pr.id);
  return `<div class="pline ${pr.produced ? 'done' : ''}">
    <div class="pline-top">${arenaIcon(ev)}<span class="t">${pr.produced ? `${CHECK} ` : ''}${ev.name}</span>${pr.produced ? bonusBadge() : ladder(ev, opts.short)}</div>
    ${pr.produced || opts.noNeeds ? '' : needRow(ev, have, { size: opts.tokSize || 'sm' })}</div>`;
}
function logHtml() {
  return `<div class="log">${STATE.log.map((e) => `<div class="e ${e.k} ${e.n ? 'new' : ''}">${e.t}</div>`).join('')}</div>`;
}

// ---------- programs for sale (content shared by every mockup's overlay) ----------
let progFilter = 'all';
function programsHtml() {
  const me = STATE.players[STATE.me];
  const cards = STATE.forSale.map(event).filter((ev) => {
    if (progFilter === 'afford') return ev.cost <= me.coins;
    if (progFilter === 'arena') return RULES.arenaBySize[ev.size] <= me.arena.len;
    return true;
  }).map((ev) => {
    const got = matched(ev, me.assets);
    const dim = ev.cost > me.coins;
    return `<button class="pcard ${dim ? 'dim' : ''}" title="${dim ? 'Too expensive' : 'Buy for ' + ev.cost}">
      <span class="top"><span class="t">${ev.name}</span><span class="cost">${icon('coins')}${ev.cost}</span></span>
      <span class="meta">${arenaIcon(ev)}${ladder(ev)}<span class="have">you hold <b>${got}</b>/${ev.requirements.length}</span></span>
      ${needRow(ev, me.assets, { size: 'sm' })}</button>`;
  }).join('');
  const chip = (k, label) => `<button class="chip ${progFilter === k ? 'on' : ''}" data-filter="${k}">${label}</button>`;
  return `<div class="prog-head"><h2>Programs for sale</h2><span class="muted">${STATE.forSale.length} left</span><span class="spacer"></span>
      <span class="chips">${chip('all', 'All')}${chip('afford', `Affordable (${icon('coins')} ${me.coins})`)}${chip('arena', `Fits arena ${me.arena.len}`)}</span></div>
    <div class="prog-grid">${cards}</div>
    <div class="prog-foot"><span>Faded tokens: ones you don't hold yet</span><span>Buying is only possible during investing</span></div>`;
}
// Wire a programs container: P toggles, Esc closes, chips filter.
function wirePrograms(el, toggle) {
  const draw = () => { el.querySelector('[data-programs]').innerHTML = programsHtml(); };
  draw();
  el.addEventListener('click', (e) => {
    const c = e.target.closest('[data-filter]');
    if (c) { progFilter = c.dataset.filter; draw(); }
  });
  document.addEventListener('keydown', (e) => {
    if (e.target.matches?.('input, select, textarea')) return;
    if (e.key === 'p' || e.key === 'P') toggle();
    if (e.key === 'Escape') toggle(false);
  });
}
function mockTag(n, name) {
  // Kept as a no-op hook: the page title names the mockup, and index.html links them all.
  void n; void name;
}
