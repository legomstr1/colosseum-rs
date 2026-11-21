// Round 4 mockup helpers (load after shared.js, round2.js and round3.js):
// Roman-numeral dice, program numbers on cards, the trade stage in five
// states, and the Stage Bench v2 seats.
'use strict';

// ---------- dice with Roman numerals ----------
const ROMAN = ['', 'I', 'II', 'III', 'IV', 'V', 'VI'];
const romanFace = (face) => (Array.isArray(face) ? `${ROMAN[Math.min(...face)]}–${ROMAN[Math.max(...face)]}` : ROMAN[face]);
// Replaces round3's pip faces.
function dieSvg(face) {
  const txt = romanFace(face);
  return `<span class="roman ${txt.length > 3 ? 'long' : ''}">${txt}</span>`;
}

// ---------- URL parameters, so the gallery can frame exact states ----------
const PARAMS = new URLSearchParams(location.search);
if (PARAMS.get('players')) setPlayers(Number(PARAMS.get('players')));
if (PARAMS.get('phase')) VIEW.phase = PARAMS.get('phase');

// ---------- trade state ----------
// mode: propose | incoming | counter | waiting | watching
const TRADE = {
  mode: PARAMS.get('trade') || 'propose',
  partner: 1,
  give: { Gladiator: 1 }, giveCoins: 0,
  get: { Musician: 1, Scenery: 1 }, getCoins: 0,
};
// The offer P3 sent you, and the trade two bots are making.
const INCOMING = { from: 2, give: { Priest: 1, Torch: 1 }, giveCoins: 0, want: { Horse: 1 }, wantCoins: 2 };
const WATCHED = { from: 1, to: 3, give: { Comedian: 2 }, giveCoins: 0, want: { Ship: 1 }, wantCoins: 3 };
const TRADE_ORDER = () => STATE.players.map((_, i) => (STATE.start + i) % STATE.n);

function subtract(have, minus) {
  const out = { ...have };
  for (const [a, n] of Object.entries(minus)) out[a] = (out[a] || 0) - n;
  return out;
}
// A hand: one tile per token in a single row, grouped by type.
function handRow(map, opts = {}) {
  const list = entries(map);
  if (!list.length) return '<span class="empty-hand"></span>';
  return `<span class="hand">${list.map(([a, n]) => `<span class="hgrp">${Array.from({ length: n }, (_, k) =>
    `<button class="tokbtn2 ${k < (opts.mark?.[a] || 0) ? 'leaving' : ''}" ${opts.attr ? `data-${opts.attr}="${a}"` : ''} title="${a}">${tok(a)}</button>`).join('')}</span>`).join('')}</span>`;
}
function coinStepper(key, val) {
  return `<span class="cstep"><button data-coin="${key}:-1">−</button><span>${icon('coins')}${val}</span><button data-coin="${key}:1">+</button></span>`;
}
// One direction of a deal: [from] → tokens and coins → [to].
function flow(from, to, assets, coins, opts = {}) {
  const items = `${entries(assets).flatMap(([a, n]) => Array.from({ length: n }, () =>
    `<button class="tokbtn2" ${opts.attr ? `data-${opts.attr}="${a}"` : ''} title="${a}">${tok(a)}</button>`)).join('')}`;
  return `<div class="flow" style="--a:${pColor(from)};--b:${pColor(to)}">
    <span class="who a">${from === 0 ? 'You' : pName(from)}</span>
    <span class="tray">${items || '<span class="slot"></span>'}${opts.coinKey ? coinStepper(opts.coinKey, coins) : coins ? `<span class="cpile">${icon('coins')}${coins}</span>` : ''}</span>
    <span class="arrow">→</span><span class="who b">${to === 0 ? 'You' : pName(to)}</span></div>`;
}
function partnerPicker() {
  return `<span class="partners">${STATE.players.map((_, i) => i).filter((i) => i !== 0).map((i) =>
    `<button class="pdot ${TRADE.partner === i ? 'on' : ''}" data-tpartner="${i}" style="--pc:${pColor(i)}">${pName(i)}</button>`).join('')}</span>`;
}
function orderStrip() {
  const cur = TRADE.mode === 'watching' ? WATCHED.from : TRADE.mode === 'incoming' || TRADE.mode === 'counter' ? INCOMING.from : 0;
  return `<span class="order">${TRADE_ORDER().map((i) => `<span class="odot ${i === cur ? 'now' : ''}" style="--pc:${pColor(i)}" title="${i === 0 ? 'You' : pName(i)}">${i === 0 ? 'You' : pName(i)}</span>`).join('<span class="osep">›</span>')}</span>`;
}
const TRADE_STAGES = {
  propose: () => {
    const p = TRADE.partner;
    return `<div class="tstage">
      <div class="trow top">${partnerPicker()}<span class="spacer"></span>${orderStrip()}</div>
      <div class="hand-line them" style="--pc:${pColor(p)}">${handRow(subtract(STATE.players[p].assets, TRADE.get), { attr: 'tget' })}</div>
      <div class="deal">${flow(0, p, TRADE.give, TRADE.giveCoins, { attr: 'tungive', coinKey: 'give' })}${flow(p, 0, TRADE.get, TRADE.getCoins, { attr: 'tunget', coinKey: 'get' })}</div>
      <div class="hand-line mine" style="--pc:var(--p0)">${handRow(subtract(STATE.players[0].assets, TRADE.give), { attr: 'tgive' })}</div>
      <div class="trow"><span class="spacer"></span><button class="btn ghost">Done trading</button><button class="btn primary big" data-send>Offer</button></div></div>`;
  },
  incoming: () => {
    const f = INCOMING.from;
    return `<div class="tstage">
      <div class="trow top"><span class="from-chip" style="--pc:${pColor(f)}">${pName(f)}</span><span class="spacer"></span>${orderStrip()}</div>
      <div class="hand-line them dim" style="--pc:${pColor(f)}">${handRow(STATE.players[f].assets, { mark: INCOMING.give })}</div>
      <div class="deal big">${flow(f, 0, INCOMING.give, INCOMING.giveCoins)}${flow(0, f, INCOMING.want, INCOMING.wantCoins)}</div>
      <div class="hand-line mine dim" style="--pc:var(--p0)">${handRow(STATE.players[0].assets, { mark: INCOMING.want })}</div>
      <div class="trow"><span class="spacer"></span><button class="round-btn no" title="Reject">✕</button><button class="round-btn counter" data-tmode="counter" title="Counter">↺</button><button class="round-btn yes" title="Accept">✓</button></div></div>`;
  },
  counter: () => {
    const f = INCOMING.from;
    // Prefilled from the offer; the player changed it to ask for coins instead of giving them.
    const give = { Horse: 1 }; const get = { Priest: 1, Torch: 1 };
    return `<div class="tstage">
      <div class="trow top"><span class="from-chip" style="--pc:${pColor(f)}">↺ ${pName(f)}</span>
        <span class="was" title="Their offer">${flow(f, 0, INCOMING.give, INCOMING.giveCoins)}${flow(0, f, INCOMING.want, INCOMING.wantCoins)}</span><span class="spacer"></span>${orderStrip()}</div>
      <div class="hand-line them" style="--pc:${pColor(f)}">${handRow(subtract(STATE.players[f].assets, get))}</div>
      <div class="deal">${flow(0, f, give, 0, { coinKey: 'give' })}${flow(f, 0, get, 0, { coinKey: 'get' })}</div>
      <div class="hand-line mine" style="--pc:var(--p0)">${handRow(subtract(STATE.players[0].assets, give))}</div>
      <div class="trow"><span class="spacer"></span><button class="btn ghost" data-tmode="incoming">Back</button><button class="btn primary big">Counter</button></div></div>`;
  },
  waiting: () => {
    const p = TRADE.partner;
    return `<div class="tstage">
      <div class="trow top"><span class="spacer"></span>${orderStrip()}</div>
      <div class="waitwho" style="--pc:${pColor(p)}"><span class="from-chip">${pName(p)}</span><span class="dots"><i></i><i></i><i></i></span></div>
      <div class="deal big">${flow(0, p, TRADE.give, TRADE.giveCoins)}${flow(p, 0, TRADE.get, TRADE.getCoins)}</div>
      <div class="hand-line mine dim" style="--pc:var(--p0)">${handRow(STATE.players[0].assets, { mark: TRADE.give })}</div></div>`;
  },
  watching: () => {
    const w = WATCHED;
    return `<div class="tstage">
      <div class="trow top"><span class="spacer"></span>${orderStrip()}</div>
      <div class="deal big watch">${flow(w.from, w.to, w.give, w.giveCoins)}${flow(w.to, w.from, w.want, w.wantCoins)}</div>
      <div class="waitwho" style="--pc:${pColor(w.to)}"><span class="from-chip">${pName(w.to)}</span><span class="dots"><i></i><i></i><i></i></span></div></div>`;
  },
};
STAGES3.Trading = () => TRADE_STAGES[TRADE.mode]();

// ---------- produce cards with the program number ----------
const baseProduceCard = produceCard;
produceCard = (pr) => baseProduceCard(pr).replace('<span class="pt">', `<span class="pt"><span class="pno sm">${pr.id}</span>`);
// Program pills that lead with the program number.
const numPills = (p) => `<span class="ppills">${p.programs.map((pr) => { const ev = event(pr.id); return `<span class="ppill ${pr.produced ? 'done' : ''}" title="${ev.id} · ${ev.name}"><span class="pno xs">${ev.id}</span>${pr.produced ? CHECK : arenaIcon(ev)}</span>`; }).join('')}</span>`;

// ---------- Stage Bench v2 seats ----------
// Your seat is wide and lists programs by name; opponents are compact and
// open a full card on hover or click (an idea from Open Floor).
function mySeat() {
  const p = STATE.players[0];
  return `<div class="seat4 me ${STATE.current === 0 ? 'turn' : ''}" style="--pc:var(--p0)">
    <div class="col-a"><div class="sh"><span class="dot"></span><b>You</b><span class="spacer"></span><span class="best">${icon('spectators')}${p.best}</span></div>
      <div class="sm"><span class="coins">${icon('coins')}${p.coins}</span><span class="ex">${extrasIcons(p)}</span></div>${seatToks(0, 'sm')}</div>
    <div class="col-b">${p.programs.map((pr) => { const ev = event(pr.id); return `<div class="prow ${pr.produced ? 'done' : ''}" title="${ev.name}"><span class="pno sm">${ev.id}</span>${arenaIcon(ev)}<b>${ev.name}</b>${pr.produced ? CHECK : `<span class="sc">${icon('spectators')}${ev.base_score}</span>`}</div>`; }).join('')}</div></div>`;
}
function oppSeat(i) {
  const p = STATE.players[i];
  const trading = VIEW.phase === 'Trading' && ((TRADE.mode === 'propose' || TRADE.mode === 'waiting') && TRADE.partner === i
    || (TRADE.mode === 'incoming' || TRADE.mode === 'counter') && INCOMING.from === i
    || TRADE.mode === 'watching' && (WATCHED.from === i || WATCHED.to === i));
  return `<div class="seat4 opp ${trading ? 'partner' : ''}" style="--pc:${pColor(i)}" ${VIEW.phase === 'Trading' && TRADE.mode === 'propose' ? `data-tpartner="${i}"` : ''}>
    <div class="sh"><span class="dot"></span><b>${pName(i)}</b><span class="muted kind">${p.seat.replace(' bot', '')}</span><span class="spacer"></span><span class="best">${icon('spectators')}${p.best}</span></div>
    <div class="sm"><span class="coins">${icon('coins')}${p.coins}</span><span class="ex">${extrasIcons(p)}</span><span class="spacer"></span>${numPills(p)}</div>
    ${seatToks(i, 'xs')}
    <div class="pop">${p.programs.map((pr) => { const ev = event(pr.id); return `<div class="prow ${pr.produced ? 'done' : ''}"><span class="pno sm">${ev.id}</span>${arenaIcon(ev)}<b>${ev.name}</b>${pr.produced ? CHECK : `<span class="sc">${icon('spectators')}${ev.base_score}</span>`}</div>`; }).join('')}</div></div>`;
}

// ---------- mockup controls: players, plus trade state while trading ----------
const TRADE_MODES = [['propose', 'Propose'], ['incoming', 'Incoming'], ['counter', 'Counter'], ['waiting', 'Waiting'], ['watching', 'Watching']];
function mockControls() {
  const trade = VIEW.phase === 'Trading' ? `<span class="msep"></span>${TRADE_MODES.map(([k, l]) => `<button data-tmode="${k}" class="${TRADE.mode === k ? 'on' : ''}">${l}</button>`).join('')}` : '';
  return `<div class="mockctl" title="Mockup controls, not part of the design"><span>mockup</span>
    ${[3, 4, 5].map((n) => `<button data-players="${n}" class="${STATE.n === n ? 'on' : ''}">${n}p</button>`).join('')}${trade}</div>`;
}
document.addEventListener('click', (e) => {
  const hit = (sel) => e.target.closest(sel);
  let el;
  if ((el = hit('[data-tmode]'))) { TRADE.mode = el.dataset.tmode; VIEW.phase = 'Trading'; }
  else if ((el = hit('[data-tpartner]'))) { TRADE.partner = Number(el.dataset.tpartner); TRADE.get = {}; TRADE.getCoins = 0; }
  else if ((el = hit('[data-tgive]'))) { const a = el.dataset.tgive; TRADE.give[a] = (TRADE.give[a] || 0) + 1; }
  else if ((el = hit('[data-tungive]'))) { const a = el.dataset.tungive; TRADE.give[a] -= 1; if (!TRADE.give[a]) delete TRADE.give[a]; }
  else if ((el = hit('[data-tget]'))) { const a = el.dataset.tget; TRADE.get[a] = (TRADE.get[a] || 0) + 1; }
  else if ((el = hit('[data-tunget]'))) { const a = el.dataset.tunget; TRADE.get[a] -= 1; if (!TRADE.get[a]) delete TRADE.get[a]; }
  else if ((el = hit('[data-coin]'))) { const [k, d] = el.dataset.coin.split(':'); const key = k === 'give' ? 'giveCoins' : 'getCoins'; TRADE[key] = Math.max(0, TRADE[key] + Number(d)); }
  else if (hit('[data-send]')) { TRADE.mode = 'waiting'; }
  else return;
  redraw();
});
// Players can change underneath the trade; keep the partner valid.
document.addEventListener('click', (e) => { if (e.target.closest('[data-players]') && TRADE.partner >= STATE.n) { TRADE.partner = 1; redraw(); } });
