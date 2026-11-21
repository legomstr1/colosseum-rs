// Round 3 mockup helpers (load after shared.js and round2.js):
// - sample states for 3, 4 and 5 players,
// - a ring board with square spaces that picks the shape (10×10 … 14×6)
//   that gives the biggest squares in the space it has,
// - quiet, icon-led stages for each phase.
'use strict';

// ---------- sample states by player count ----------
const PROFILES = [
  { seat: 'You', len: 3, coins: 23, loge: true, tickets: 1, medals: 2, podiums: 0, best: 18,
    assets: { Gladiator: 4, Horse: 3, Ship: 1, Lion: 1, Scenery: 1, Decoration: 2, Joker: 1 },
    programs: [{ id: 4, produced: true }, { id: 10 }, { id: 13 }] },
  { seat: 'Oracle bot', len: 2, coins: 31, loge: false, tickets: 0, medals: 1, podiums: 0, best: 12,
    assets: { Comedian: 4, Musician: 3, Priest: 2, Torch: 1, Scenery: 2 }, programs: [{ id: 2, produced: true }, { id: 9 }, { id: 14 }] },
  { seat: 'Maestro bot', len: 3, coins: 17, loge: false, tickets: 2, medals: 0, podiums: 1, best: 23,
    assets: { Musician: 2, Priest: 3, Torch: 2, Chariot: 1, Decoration: 1 }, programs: [{ id: 3, produced: true }, { id: 8, produced: true }, { id: 17 }] },
  { seat: 'Greedy bot', len: 2, coins: 40, loge: false, tickets: 0, medals: 0, podiums: 0, best: 9,
    assets: { Gladiator: 3, Ship: 3, Lion: 2, Cage: 2 }, programs: [{ id: 5, produced: true }, { id: 12 }] },
  { seat: 'Planner bot', len: 2, coins: 26, loge: false, tickets: 1, medals: 1, podiums: 0, best: 7,
    assets: { Comedian: 2, Horse: 2, Chariot: 2, Scenery: 1, Torch: 1 }, programs: [{ id: 1, produced: true }, { id: 6 }, { id: 15 }] },
];
const ARENA_STARTS = { 3: [8, 21, 32], 4: [3, 14, 23, 32], 5: [2, 8, 15, 23, 32] };
const STAR_HOLDERS = { Gladiator: 0, Horse: 0, Musician: 1, Comedian: 1, Priest: 2, Ship: 3, Lion: null };
function setPlayers(n) {
  STATE.players = PROFILES.slice(0, n).map((p, i) => ({ ...p, arena: { start: ARENA_STARTS[n][i], len: p.len } }));
  STATE.stars = Object.fromEntries(Object.entries(STAR_HOLDERS).map(([a, h]) => [a, h !== null && h < n ? h : null]));
  STATE.n = n;
}
setPlayers(4);

// ---------- view state for the mockups ----------
const VIEW = { phase: 'Producing', market: 1, partner: 1, bid: 8 };

// ---------- dice drawn with pips ----------
const PIPS = { 1: [[1, 1]], 2: [[0, 0], [2, 2]], 3: [[0, 0], [1, 1], [2, 2]], 4: [[0, 0], [0, 2], [2, 0], [2, 2]],
  5: [[0, 0], [0, 2], [1, 1], [2, 0], [2, 2]], 6: [[0, 0], [0, 2], [1, 0], [1, 2], [2, 0], [2, 2]] };
function dieSvg(face) {
  if (Array.isArray(face)) {
    // The choice face: one, two and three pips side by side.
    const col = (n, x) => Array.from({ length: n }, (_, k) => `<circle cx="${x}" cy="${50 + (k - (n - 1) / 2) * 22}" r="7.5"/>`).join('');
    return `<svg viewBox="0 0 100 100" class="pipface">${col(1, 22)}${col(2, 50)}${col(3, 78)}<path d="M36 22v56M64 22v56" stroke="#c9bda6" stroke-width="2.5" stroke-linecap="round"/></svg>`;
  }
  return `<svg viewBox="0 0 100 100" class="pipface">${PIPS[face].map(([r, c]) => `<circle cx="${24 + c * 26}" cy="${24 + r * 26}" r="9"/>`).join('')}</svg>`;
}
function bigDice() {
  const d = (k) => `<button class="bigdie ${MOVE.die === k || MOVE.die === 'both' ? 'on' : ''}" data-die="${k}" title="${k === '0' ? 'Move 1, 2 or 3' : 'Move 5'}">${dieSvg(STATE.dice[Number(k)])}</button>`;
  return `<div class="dicepair">${d('0')}<button class="link ${MOVE.die === 'both' ? 'on' : ''}" data-die="both" title="Use both dice on one noble">+</button>${d('1')}</div>`;
}

// ---------- square-space ring that picks its own shape ----------
const SHAPES = [[10, 10], [11, 9], [12, 8], [13, 7], [14, 6], [9, 11], [8, 12]];
function fitShape(W, H, g) {
  let best = null;
  for (const [w, h] of SHAPES) {
    const s = Math.floor(Math.min((W - (w - 1) * g) / w, (H - (h - 1) * g) / h));
    if (!best || s > best.s) best = { w, h, s };
  }
  best.s = Math.min(best.s, 130);
  return best;
}
function ringHtml(W, H, inner) {
  const g = 5;
  const { w, h, s } = fitShape(W, H, g);
  const targets = VIEW.phase === 'Producing' ? moveTargets() : {};
  const extra = VIEW.phase === 'Investing' ? { [(STATE.players[0].arena.start + STATE.players[0].arena.len) % RULES.board]: '<span class="grow" title="Expand your arena here (10 coins)">+</span>' } : {};
  const rest = new Set(RULES.rests);
  const at = {};
  STATE.nobles.forEach((nb, i) => (at[nb.position] ||= []).push([nb, i]));
  let cells = '';
  for (let sq = 0; sq < RULES.board; sq++) {
    const [r, c] = rectPos(sq, w, h);
    const owner = arenaOwner(sq);
    const tg = targets[sq];
    const cls = ['sq', rest.has(sq) ? 'rest' : '', owner !== null ? 'arena' : '', tg ? 'target' : '', extra[sq] ? 'grow-sq' : ''].join(' ');
    const nobles = (at[sq] || []).map(([nb, i]) => `<span class="noble ${MOVE.noble === i ? 'sel' : ''} ${VIEW.phase === 'Producing' ? 'live' : ''}" data-noble="${i}" style="--nc:${NOBLES[nb.kind].bg};color:${NOBLES[nb.kind].fg}" title="${nb.kind}">${icon(nb.kind)}</span>`).join('');
    const ghosts = (tg || []).map(({ i, d }) => { const nb = STATE.nobles[i]; return `<span class="ghost" data-move="${i}:${d}:${sq}" title="${nb.kind} → ${sq}"><span class="noble" style="--nc:${NOBLES[nb.kind].bg};color:${NOBLES[nb.kind].fg}">${icon(nb.kind)}</span></span>`; }).join('');
    cells += `<div class="${cls}" style="grid-row:${r + 1};grid-column:${c + 1};${owner !== null ? `--pc:${pColor(owner)}` : ''}"><span class="num">${sq}</span>${nobles}${ghosts}${extra[sq] || ''}</div>`;
  }
  return `<div class="ring" style="--s:${s}px;--g:${g}px;grid-template-columns:repeat(${w},${s}px);grid-template-rows:repeat(${h},${s}px)">${cells}
    <div class="stage-wrap" style="grid-row:2/${h};grid-column:2/${w}">${inner()}</div></div>`;
}
// Render a ring into `el`, re-rendering when it changes size.
function mountRing3(el, inner) {
  const draw = () => { const r = el.getBoundingClientRect(); el.innerHTML = ringHtml(r.width, r.height, inner); };
  new ResizeObserver(draw).observe(el);
  BOARDS.push(draw);
  draw();
}

// ---------- player seats ----------
function seatToks(i, size = 'xs') {
  const p = STATE.players[i];
  const list = entries(p.assets);
  return `<span class="toks ptoks">${list.map(([a, n]) => `<span class="tcol ${STATE.stars[a] === i ? 'starcol' : ''}" title="${a}${STATE.stars[a] === i ? ' · star' : ''}">${Array.from({ length: n }, () => tok(a, { size })).join('')}</span>`).join('')}</span>`;
}
function progPills(p) {
  return `<span class="ppills">${p.programs.map((pr) => { const ev = event(pr.id); return `<span class="ppill ${pr.produced ? 'done' : ''}" title="${ev.name}${pr.produced ? ' (produced)' : ` · ${ev.base_score}`}">${arenaIcon(ev)}${pr.produced ? CHECK : `<b>${ev.base_score}</b>`}</span>`; }).join('')}</span>`;
}
function extrasIcons(p) {
  return `${p.loge ? `<span title="Emperor's loge">${icon('loge')}</span>` : ''}${repeatIcon('ticket', p.tickets)}${repeatIcon('medal', p.medals)}${repeatIcon('podium', p.podiums)}`;
}
function seat(i, opts = {}) {
  const p = STATE.players[i];
  return `<div class="seat3 ${i === STATE.current ? 'turn' : ''} ${opts.cls || ''}" style="--pc:${pColor(i)}">
    <div class="sh"><span class="dot"></span><b>${i === 0 ? 'You' : pName(i)}</b>${i === 0 ? '' : `<span class="muted kind">${p.seat.replace(' bot', '')}</span>`}<span class="spacer"></span>
      <span class="best" title="Best event so far">${icon('spectators')}${p.best}</span></div>
    <div class="sm"><span class="coins" title="Coins">${icon('coins')}${p.coins}</span><span class="ex">${extrasIcons(p)}</span><span class="spacer"></span>${progPills(p)}</div>
    ${seatToks(i, opts.tok || 'xs')}</div>`;
}

// ---------- stages: what the middle of the ring shows in each phase ----------
const me3 = () => STATE.players[0];
function produceCard(pr) {
  const ev = event(pr.id);
  const got = matched(ev, me3().assets);
  const missing = ev.requirements.length - got;
  const cells = [ev.base_score, ...ev.penalty_scores];
  const score = missing < cells.length ? cells[missing] : null;
  return `<button class="pcard3" data-produce title="Produce ${ev.name}">
    <span class="pt">${arenaIcon(ev)}<b>${ev.name}</b></span>
    <span class="falloff">${cells.map((v, k) => `<span class="${k === missing ? 'cur' : ''} ${k ? '' : 'base'}">${v}</span>`).join('')}</span>
    ${needRow(ev, me3().assets, { size: 'sm' })}
    <span class="pscore">${icon('spectators')}${score ?? '–'}</span></button>`;
}
const STAGES3 = {
  Investing: () => {
    const opt = (ic, label, price, attrs = '', off = false) => `<button class="opt3 ${off ? 'off' : ''}" ${attrs} title="${label}"><span class="oi">${ic}</span><span>${label}</span>${price ? `<span class="price">${icon('coins')}${price}</span>` : ''}</button>`;
    return `<div class="st-row">
      ${opt(`<span class="arena-big">${arenaIcon({ size: 2 })}</span>`, 'Program', '13+', 'data-open-programs')}
      ${opt('<span class="plus">+</span>', 'Arena', 10)}
      ${opt(icon('ticket'), 'Season ticket', 10)}
      ${opt(icon('loge'), 'Loge', 5, '', true)}
      ${opt('<span class="plus">✓</span>', 'Pass', '')}</div>
      <div class="purse">${icon('coins')}<b>${me3().coins}</b></div>`;
  },
  Acquiring: () => `<div class="st-row">${STATE.markets.map((m, i) => `<button class="mk3 ${VIEW.market === i ? 'on' : ''} ${m.length ? '' : 'empty'}" data-market="${i}">${m.map((a) => tok(a)).join('')}</button>`).join('')}</div>
    <div class="st-row center"><span class="stepper big"><button data-bid="-1">−</button><span>${icon('coins')}${VIEW.bid}</span><button data-bid="1">+</button></span><button class="btn primary big">Open auction</button>
      <span class="bag" title="Tokens left in the bag">${STATE.bag}</span></div>`,
  Trading: () => {
    const partners = STATE.players.map((_, i) => i).filter((i) => i !== 0);
    return `<div class="trade3">
      <div class="tside">${seatToks(0, '').replace('class="tok ', 'class="tok picked ')}<span class="stepper"><button>−</button><span>${icon('coins')}0</span><button>+</button></span></div>
      <div class="tmid"><span class="swap">⇄</span><span class="partners">${partners.map((i) => `<button class="pdot ${VIEW.partner === i ? 'on' : ''}" data-partner="${i}" style="--pc:${pColor(i)}" title="${pName(i)}">${pName(i)}</button>`).join('')}</span><button class="btn primary">Offer</button></div>
      <div class="tside">${seatToks(VIEW.partner, '')}<span class="stepper"><button>−</button><span>${icon('coins')}3</span><button>+</button></span></div></div>`;
  },
  Producing: () => `<div class="prod3">${bigDice()}<div class="pcards">${me3().programs.filter((p) => !p.produced).map(produceCard).join('')}</div></div>`,
  Closing: () => {
    const order = STATE.players.map((p, i) => [i, [30, 17, 12, 0, 9][i]]).sort((a, b) => b[1] - a[1]);
    return `<div class="podium3">${order.map(([i, s], k) => `<div class="step" style="--pc:${pColor(i)};--h:${Math.max(18, 100 - k * 22)}%"><span class="who">${i === 0 ? 'You' : pName(i)}</span><span class="bar">${k === 0 ? icon('podium') : ''}<b>${s || '–'}</b></span></div>`).join('')}</div>
      <div class="st-row center"><span class="muted">Discard one</span>${['Gladiator', 'Horse', 'Joker'].map((a) => `<button class="tokbtn">${tok(a)}</button>`).join('')}</div>`;
  },
};
function stageHtml() {
  return `<div class="stage3 phase-${VIEW.phase}" style="--pc:${pColor(STATE.current)}">${STAGES3[VIEW.phase]()}</div>`;
}

// ---------- mockup-only controls (player count, phase preview) ----------
function mockControls() {
  return `<div class="mockctl" title="Mockup controls, not part of the design"><span>mockup</span>
    ${[3, 4, 5].map((n) => `<button data-players="${n}" class="${STATE.n === n ? 'on' : ''}">${n}p</button>`).join('')}</div>`;
}
document.addEventListener('click', (e) => {
  const t = e.target;
  const hit = (sel) => t.closest(sel);
  if (hit('[data-players]')) { setPlayers(Number(hit('[data-players]').dataset.players)); MOVE.noble = null; redraw(); }
  else if (hit('[data-phase]')) { VIEW.phase = hit('[data-phase]').dataset.phase; MOVE.noble = null; redraw(); }
  else if (hit('[data-market]')) { VIEW.market = Number(hit('[data-market]').dataset.market); redraw(); }
  else if (hit('[data-bid]')) { VIEW.bid = Math.max(8, VIEW.bid + Number(hit('[data-bid]').dataset.bid)); redraw(); }
  else if (hit('[data-partner]')) { VIEW.partner = Number(hit('[data-partner]').dataset.partner); redraw(); }
  else if (hit('[data-open-programs]')) document.getElementById('pmodal')?.classList.add('open');
});

// ---------- page chrome shared by the round 3 pages ----------
const topButtons = () => `<button class="iconbtn" id="open" title="Programs for sale (P)">Programs <kbd>P</kbd></button>
  <button class="iconbtn" id="logbtn" title="Log (L)">Log <kbd>L</kbd></button>`;
function mountChrome() {
  const toggleP = mountProgramModal([]);
  document.body.insertAdjacentHTML('beforeend', `<div class="drawer3" id="drawer"><div class="row between" style="margin-bottom:8px"><b>Log</b><kbd>L</kbd></div>${logHtml()}</div><div id="mockctl"></div>`);
  const d = document.getElementById('drawer');
  // Delegated, because the top bar is redrawn.
  document.addEventListener('click', (e) => {
    if (e.target.closest('#open')) toggleP(true);
    if (e.target.closest('#logbtn')) d.classList.toggle('open');
  });
  document.addEventListener('keydown', (e) => { if (e.key === 'l' || e.key === 'L') d.classList.toggle('open'); if (e.key === 'Escape') d.classList.remove('open'); });
  BOARDS.push(() => { document.getElementById('mockctl').innerHTML = mockControls(); });
  document.getElementById('mockctl').innerHTML = mockControls();
}
