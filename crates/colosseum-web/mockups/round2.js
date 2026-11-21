// Round 2 mockup helpers (load after shared.js): a rectangular ring board you
// act on directly, the round/phase tracker, and the programs-for-sale list
// with the full spectator fall-off.
'use strict';

const PHASES = ['Investing', 'Acquiring', 'Trading', 'Producing', 'Closing'];

// ---------- tracker ----------
function trackerHtml(opts = {}) {
  const now = PHASES.indexOf(opts.phase || STATE.phase);
  const pips = Array.from({ length: RULES.rounds }, (_, i) =>
    `<span class="${i + 1 < STATE.round ? 'done' : i + 1 === STATE.round ? 'now' : ''}"></span>`).join('');
  const tag = opts.clickable ? 'button' : 'span';
  const phases = PHASES.map((p, i) =>
    `<${tag} class="ph ${i < now ? 'done' : i === now ? 'now' : ''}" ${opts.clickable ? `data-phase="${p}"` : ''}><i></i>${p}</${tag}>`).join('');
  return `<div class="tracker ${opts.inline ? 'inline' : ''}">
    <div class="rnd"><span class="big">Round ${STATE.round}<small> / ${RULES.rounds}</small></span><span class="pips">${pips}</span></div>
    <div class="phases">${phases}</div></div>`;
}

// ---------- programs for sale: list with full fall-off ----------
function falloff(ev) {
  const cells = [ev.base_score, ...ev.penalty_scores];
  return `<span class="falloff">${Array.from({ length: 7 }, (_, i) => (i < cells.length
    ? `<span class="${i ? '' : 'base'}" title="${i} missing: ${cells[i]} spectators">${cells[i]}</span>`
    : '<span class="none"></span>')).join('')}</span>`;
}
function programListHtml() {
  const me = STATE.players[STATE.me];
  const rows = STATE.forSale.map(event).map((ev) => `<div class="plist-row ${ev.cost > me.coins ? 'dim' : ''}">
      <span class="pno" title="Program ${ev.id}">${ev.id}</span>
      <span class="cost">${icon('coins')}${ev.cost}</span>
      <span class="t">${ev.name}${arenaIcon(ev)}</span>
      ${falloff(ev)}
      <span class="hold">hold <b>${matched(ev, me.assets)}</b>/${ev.requirements.length}</span>
      ${needRow(ev, me.assets)}</div>`).join('');
  return `<div class="plist-wrap">
    <div class="plist-head"><span>No.</span><span>Cost</span><span>Program</span><span class="falloff head">${[0, 1, 2, 3, 4, 5, 6].map((n) => `<span>${n}</span>`).join('')}</span><span></span></div>
    <div class="plist-head" style="padding-top:0;border:0;margin-top:-4px"><span></span><span></span><span></span><span style="text-transform:none;letter-spacing:0">${icon('spectators')} spectators by tokens missing</span><span></span></div>
    <div class="plist-body">${rows}</div>
    <div class="plist-foot"><span>Faded tokens: ones you don't hold</span><span>Faded rows: more than your ${me.coins} coins</span><span class="spacer"></span><span><kbd>P</kbd> / <kbd>Esc</kbd> close</span></div></div>`;
}
// A modal holding the list; P toggles, Esc closes.
function mountProgramModal(openers = []) {
  document.body.insertAdjacentHTML('beforeend', `<div class="modal" id="pmodal"><div class="panel">
    <div class="mhead"><h2>Programs for sale</h2><span class="muted">${STATE.forSale.length} left · buy during investing</span><span class="spacer"></span><button class="btn small ghost" data-close>Close</button></div>
    ${programListHtml()}</div></div>`);
  const m = document.getElementById('pmodal');
  const toggle = (v) => m.classList.toggle('open', v);
  m.addEventListener('click', (e) => { if (e.target === m || e.target.closest('[data-close]')) toggle(false); });
  openers.forEach((id) => document.getElementById(id)?.addEventListener('click', () => toggle(true)));
  document.addEventListener('keydown', (e) => {
    if (e.target.matches?.('input, select, textarea')) return;
    if (e.key === 'p' || e.key === 'P') toggle(!m.classList.contains('open'));
    if (e.key === 'Escape') toggle(false);
  });
  return toggle;
}

// ---------- player bits (tokens are always tiles) ----------
const repeatIcon = (name, n) => (n ? `<span class="icons-n" title="${n} ${name}">${Array.from({ length: n }, () => icon(name)).join('')}</span>` : '');
function statsLine(p) {
  return `<span class="stats-line"><span>${icon('coins')} <b>${p.coins}</b></span><span>arena <b>${p.arena.len}</b></span>
    ${p.loge ? `<span>${icon('loge')} loge</span>` : ''}${repeatIcon('ticket', p.tickets)}${repeatIcon('medal', p.medals)}${repeatIcon('podium', p.podiums)}</span>`;
}
const starTiles = (i, size = 'sm') => starsOf(i).map((a) => `<span class="star">${tok(a, { size })}</span>`).join('');
const playerToks = (p, size = 'sm') => `<span class="ptoks">${toks(p.assets, { size })}</span>`;

// ---------- the ring board, played on directly ----------
// Pick a die (or both), then click a noble to see where it can go, or click a
// faded noble on a target square to move it there.
const MOVE = { die: '1', noble: null };
const dieDistances = (k) => (k === '0' ? [1, 2, 3] : k === '1' ? [5] : [6, 7, 8]);
function moveTargets() {
  const out = {};
  STATE.nobles.forEach((nb, i) => {
    if (MOVE.noble !== null && MOVE.noble !== i) return;
    for (const d of dieDistances(MOVE.die)) (out[(nb.position + d) % RULES.board] ||= []).push({ i, d });
  });
  return out;
}
function rectPos(i, w, h) {
  if (i < w - 1) return [0, i];
  if (i < w - 1 + h - 1) return [i - (w - 1), w - 1];
  if (i < 2 * (w - 1) + h - 1) return [h - 1, (w - 1) - (i - (w - 1) - (h - 1))];
  return [(h - 1) - (i - 2 * (w - 1) - (h - 1)), 0];
}
function diceButtons() {
  const label = (k) => (k === 'both' ? 'Both' : faceLabel(STATE.dice[Number(k)]));
  return `<span class="dice">${['0', '1', 'both'].map((k) => `<button class="die ${MOVE.die === k ? 'on' : ''}" data-die="${k}">${label(k)}</button>`).join('')}</span>`;
}
function moveHint() {
  if (MOVE.noble === null) return 'Click a noble on the board, or a faded noble on a dashed square to move it there.';
  const nb = STATE.nobles[MOVE.noble];
  return `${nb.kind} on ${nb.position} selected. Click a dashed square to move it.`;
}
const BOARDS = [];
// `inner` is a list of {html, row, col} placed inside the ring (grid lines),
// or one function returning HTML for the whole middle.
function mountRing(el, w, h, inner) {
  const draw = () => {
    // A page can switch the ring out of noble-move mode (window.boardMoves =
    // false) and mark its own squares (window.boardExtra() -> {sq: html}).
    const targets = window.boardMoves === false ? {} : moveTargets();
    const extra = window.boardExtra ? window.boardExtra() : {};
    const rest = new Set(RULES.rests);
    const at = {};
    STATE.nobles.forEach((nb, i) => (at[nb.position] ||= []).push([nb, i]));
    let cells = '';
    for (let sq = 0; sq < RULES.board; sq++) {
      const [r, c] = rectPos(sq, w, h);
      const owner = arenaOwner(sq);
      const tg = targets[sq];
      const cls = ['sq', rest.has(sq) ? 'rest' : '', owner !== null ? 'arena' : '', tg || extra[sq] ? 'target' : ''].join(' ');
      const nobles = (at[sq] || []).map(([nb, i]) => `<span class="noble ${MOVE.noble === i ? 'sel' : ''}" data-noble="${i}" style="--nc:${NOBLES[nb.kind].bg};color:${NOBLES[nb.kind].fg}" title="${nb.kind}: click to see its moves">${icon(nb.kind)}</span>`).join('');
      const ghosts = (tg || []).map(({ i, d }) => { const nb = STATE.nobles[i]; return `<span class="ghost" data-move="${i}:${d}:${sq}" title="Move ${nb.kind} +${d} to ${sq}"><span class="noble" style="--nc:${NOBLES[nb.kind].bg};color:${NOBLES[nb.kind].fg}">${icon(nb.kind)}</span>+${d}</span>`; }).join('');
      cells += `<div class="${cls}" style="grid-row:${r + 1};grid-column:${c + 1};${owner !== null ? `--pc:${pColor(owner)}` : ''}"><span class="num">${sq}</span>${nobles}${ghosts}${extra[sq] || ''}${owner !== null ? `<span class="own">${pName(owner)}</span>` : ''}</div>`;
    }
    const mid = typeof inner === 'function'
      ? `<div class="inner" style="grid-row:2/${h};grid-column:2/${w}">${inner()}</div>`
      : inner.map((b) => `<div class="inner ${b.cls || ''}" style="grid-row:${b.row};grid-column:${b.col};${b.style || ''}">${typeof b.html === 'function' ? b.html() : b.html}</div>`).join('');
    el.innerHTML = `<div class="rboard" style="--w:${w};--h:${h}">${cells}${mid}</div>`;
  };
  BOARDS.push(draw);
  draw();
}
const redraw = () => BOARDS.forEach((d) => d());
function toast2(msg) {
  let t = document.querySelector('.toast2');
  if (!t) { document.body.insertAdjacentHTML('beforeend', '<div class="toast2"></div>'); t = document.querySelector('.toast2'); }
  t.textContent = msg;
  t.classList.add('show');
  clearTimeout(toast2.t);
  toast2.t = setTimeout(() => t.classList.remove('show'), 2600);
}
document.addEventListener('click', (e) => {
  const die = e.target.closest('[data-die]');
  if (die) { MOVE.die = die.dataset.die; redraw(); return; }
  const mv = e.target.closest('[data-move]');
  if (mv) {
    const [i, d, sq] = mv.dataset.move.split(':').map(Number);
    toast2(`Would send MoveNoble: ${STATE.nobles[i].kind} +${d} to square ${sq}`);
    return;
  }
  const nb = e.target.closest('[data-noble]');
  if (nb) { const i = Number(nb.dataset.noble); MOVE.noble = MOVE.noble === i ? null : i; redraw(); return; }
  if (e.target.closest('[data-produce]')) toast2('Would send ProduceEvent (after both dice are used)');
});

// The same moves as the faded nobles on the board, as a list of chips.
function moveChips() {
  const t = moveTargets();
  const chips = [];
  for (const [sq, list] of Object.entries(t)) for (const { i, d } of list) {
    const nb = STATE.nobles[i];
    const owner = arenaOwner(Number(sq));
    const tag = owner === STATE.me ? '<b>your arena</b>' : owner !== null ? `${pName(owner)}'s arena` : RULES.rests.includes(Number(sq)) ? `${icon('medal')}` : '';
    chips.push([nb.position, `<button class="chip mchip ${owner === STATE.me ? 'mine' : ''}" data-move="${i}:${d}:${sq}"><span style="color:${NOBLES[nb.kind].fg}">${icon(nb.kind)}</span> ${nb.position}→${sq} <span class="muted">${tag}</span></button>`]);
  }
  return `<span class="chips">${chips.sort((a, b) => a[0] - b[0]).map((c) => c[1]).join('')}</span>`;
}
