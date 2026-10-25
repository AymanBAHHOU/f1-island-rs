// The page side. `withGlobalTauri` in tauri.conf.json puts Tauri's API on
// window.__TAURI__, so no bundler is needed.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const island = $('island');
const expanded = $('expanded');
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

// Team colours, by Jolpica's constructor id (from the Electron app).
const TEAM_COLORS = {
  red_bull: '#3671C6', ferrari: '#E8002D', mercedes: '#27F4D2', mclaren: '#FF8000',
  aston_martin: '#229971', alpine: '#00A1E8', williams: '#64C4FF', rb: '#6692FF',
  sauber: '#52E252', audi: '#F50537', haas: '#B6BABD', cadillac: '#C8A45A',
};
const teamBar = (teamId) => `<i class="team" style="--team:${TEAM_COLORS[teamId] || '#48484a'}"></i>`;

const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: 'short' });
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
const when = (ms) => `${dayFmt.format(ms)} ${timeFmt.format(ms)}`;
const shortName = (name) => name.replace(/Grand Prix/i, 'GP');

const fmtCountdown = (ms) => {
  const s = Math.max(0, Math.floor(ms / 1000));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, '0');
  const r = String(s % 60).padStart(2, '0');
  return d ? `${d}d ${h}h` : `${h}:${m}:${r}`;
};

// A race time "1:34:00.161" written so it can't pass for a lap time: "1h 34m 00.161s".
function raceTime(t) {
  const parts = String(t ?? '').split(':');
  if (parts.length < 2) return t ?? '';
  const sec = parts.pop();
  const min = parts.pop();
  const hours = parts.pop();
  return `${hours ? `${Number(hours)}h ` : ''}${min.padStart(2, '0')}m ${sec}s`;
}

// ---------- state ----------

let weekend = null; // { season, round, name, circuitId, circuit, country, sessions: [...] }
let problem = null; // why we couldn't get it
let standings = null; // { drivers: { round, rows }, constructors, lastRace }
let standingsProblem = null;
let history = null; // { winners, last }
let historyProblem = null;
let page = 'weekend';

// ---------- asking Rust ----------

async function loadWeekend() {
  try {
    weekend = await invoke('weekend');
    problem = weekend ? null : 'Season complete';
  } catch (err) {
    problem = String(err);
  }
  render();
  if (weekend) loadCircuit(); // needs the circuit from the weekend
}

async function loadStandings() {
  try {
    standings = await invoke('standings');
    standingsProblem = null;
  } catch (err) {
    standingsProblem = String(err);
  }
  render();
}

async function loadCircuit() {
  if (!weekend) return;
  try {
    history = await invoke('circuit_history', { circuitId: weekend.circuitId, season: weekend.season });
    historyProblem = null;
  } catch (err) {
    historyProblem = String(err);
  }
  render();
}

// ---------- drawing ----------

function weekendPage(now) {
  const next = weekend.sessions.find((s) => s.endMs > now);
  if (next) {
    const live = now >= next.startMs;
    $('next').textContent = next.name;
    $('nextLabel').textContent = live ? 'Live now' : 'Next up';
    $('count').textContent = live ? 'LIVE' : fmtCountdown(next.startMs - now);
    $('cRight').innerHTML = live
      ? `<b class="live">LIVE</b> ${esc(next.short)}`
      : `${esc(next.short)} <b>${fmtCountdown(next.startMs - now)}</b>`;
  } else {
    $('next').textContent = 'Weekend complete';
    $('nextLabel').textContent = '';
    $('count').textContent = '🏁';
    $('cRight').textContent = 'Chequered flag';
  }
  $('sessions').innerHTML = weekend.sessions
    .map((s) => {
      const state = now >= s.endMs ? 'done' : now >= s.startMs ? 'live' : s === next ? 'next' : '';
      return `<li class="${state}"><span class="s-short">${esc(s.short)}</span><span class="s-when">${esc(when(s.startMs))}</span></li>`;
    })
    .join('');
}

function standingsPage() {
  if (standingsProblem && !standings) return `<div class="message">${esc(standingsProblem)}</div>`;
  if (!standings) return '<div class="message">Loading standings…</div>';

  const rows = (list, label) =>
    `<ul class="rows">${list
      .map((r) => `<li><span class="pos">${esc(r.pos)}</span>${teamBar(r.teamId)}<span class="code">${esc(label(r))}</span><span class="pts">${esc(r.points)}</span></li>`)
      .join('')}</ul>`;

  const last = standings.lastRace;
  const lastRace = last
    ? `<div class="last-race"><h2>${esc(shortName(last.name))}</h2><div class="last-podium">${last.podium
        .map((p) => `<span class="pod"><small>${esc(p.pos)}</small>${teamBar(p.teamId)}<b>${esc(p.code)}</b></span>`)
        .join('')}</div></div>`
    : '';
  const drivers = standings.drivers ? rows(standings.drivers.rows, (r) => r.code) : '<div class="message">No table yet.</div>';
  const teams = standings.constructors.length ? rows(standings.constructors, (r) => r.name) : '<div class="message">No table yet.</div>';
  return `${lastRace}<div class="cols"><section><h2>Drivers</h2>${drivers}</section><section><h2>Constructors</h2>${teams}</section></div>`;
}

function circuitPage() {
  if (historyProblem && !history) return `<div class="message">${esc(historyProblem)}</div>`;
  if (!history) return '<div class="message">Loading circuit history…</div>';
  const l = history.last;
  if (!l && !history.winners.length) return '<div class="message">No previous races at this circuit.</div>';

  const fact = (label, entry, value) =>
    entry ? `<li><span class="label">${label}</span>${teamBar(entry.teamId)}<span class="code">${esc(entry.code)}</span><span class="val">${value}</span></li>` : '';
  const recap = l
    ? `<div class="cols">
        <section><h2>Last time here · ${esc(l.season)}</h2><ul class="facts">
          ${fact('Pole lap', l.pole, esc(l.pole?.time ?? ''))}
          ${fact('Race time', l.podium[0], esc(raceTime(l.winnerTime)))}
          ${fact('Fastest lap', l.fastestLap, l.fastestLap ? `${esc(l.fastestLap.time)} <small>L${esc(l.fastestLap.lap)}</small>` : '')}
          <li><span class="label">Race laps</span><span class="val">${esc(l.laps ?? '')}</span></li>
        </ul></section>
        <section><h2>Podium</h2><ul class="rows">${l.podium
          .map((p) => `<li><span class="pos">${esc(p.pos)}</span>${teamBar(p.teamId)}<span class="code">${esc(p.code)}</span></li>`)
          .join('')}</ul></section>
      </div>`
    : '';
  const winners = history.winners.length
    ? `<h2 class="spaced">Recent winners</h2><div class="winners">${history.winners
        .map((w) => `<span><small>${esc(w.season)}</small>${teamBar(w.teamId)}<b>${esc(w.code)}</b></span>`)
        .join('')}</div>`
    : '';
  return recap + winners;
}

function render() {
  const now = Date.now();
  if (!weekend) {
    $('cLabel').textContent = 'Formula 1';
    $('cRight').textContent = problem || 'Loading…';
    $('title').textContent = problem || 'Loading…';
    return;
  }

  $('cLabel').textContent = shortName(weekend.name);
  $('eyebrow').textContent = `Round ${weekend.round} · ${weekend.season}`;
  $('title').textContent = weekend.name;
  $('subtitle').textContent = `${weekend.circuit} · ${weekend.country}`;

  weekendPage(now); // also keeps the small island's countdown moving
  if (page === 'standings') $('page-standings').innerHTML = standingsPage();
  if (page === 'circuit') $('page-circuit').innerHTML = circuitPage();
  fit();
}

// The island is as tall as what's in it, while open.
function fit() {
  island.style.height = island.classList.contains('open') ? `${expanded.offsetHeight}px` : '';
}

function show(next) {
  page = next;
  for (const b of document.querySelectorAll('#pages button')) b.classList.toggle('active', b.dataset.page === next);
  for (const name of ['weekend', 'standings', 'circuit']) $(`page-${name}`).hidden = name !== next;
  render();
}

// ---------- events ----------

// The tabs only work while the island is open, and the island is only clickable then.
$('pages').addEventListener('click', (e) => {
  const button = e.target.closest('button[data-page]');
  if (button) show(button.dataset.page);
});

// Rust watches the mouse and tells us when it is over the island.
listen('hover', (event) => {
  island.classList.toggle('open', event.payload);
  if (event.payload) show('weekend'); // it always opens on the Weekend
  fit();
});

// Tell Rust where the island is, so it knows what "over the island" means. The island
// grows and shrinks, so report whenever its zone changes size.
const zone = $('zone');
const report = () => {
  const r = zone.getBoundingClientRect();
  invoke('report_rect', { rect: { x: r.x, y: r.y, width: r.width, height: r.height } });
};
new ResizeObserver(report).observe(zone);
report();

loadWeekend();
loadStandings();
setInterval(loadWeekend, 5 * 60 * 1000);
setInterval(loadStandings, 10 * 60 * 1000);
setInterval(render, 1000);
