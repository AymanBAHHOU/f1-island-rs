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
let live = { summary: null, main: false }; // what Rust says about the live session
let page = 'weekend';
let pagePicked = false; // a tab was chosen since the island opened

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
    const isLive = now >= next.startMs;
    $('next').textContent = next.name;
    $('nextLabel').textContent = isLive ? 'Live now' : 'Next up';
    $('count').textContent = isLive ? 'LIVE' : fmtCountdown(next.startMs - now);
    $('cRight').innerHTML = isLive
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

// ---------- live timing ----------

const TYRE = { SOFT: ['S', '#ff3b30'], MEDIUM: ['M', '#ffd60a'], HARD: ['H', '#f5f5f7'], INTERMEDIATE: ['I', '#30d158'], WET: ['W', '#0a84ff'] };
const tyreCell = (t) => {
  const known = t && TYRE[t.compound];
  return known ? `<span class="tyre"><i style="--c:${known[1]}">${known[0]}</i>${esc(t.laps ?? '')}</span>` : '<span class="tyre"></span>';
};

// Leaders are reported as "LAP 12" (races) or an empty gap (practice).
const gapText = (v) => (!v || /^LAP/i.test(v) ? '' : v);

// Time left on the session clock, counted down locally between the feed's reports.
function clockText(clock, now) {
  if (!clock?.remaining) return '';
  const [h = 0, m = 0, s = 0] = clock.remaining.split(':').map(Number);
  let ms = (h * 3600 + m * 60 + s) * 1000;
  if (clock.running) ms = Math.max(0, ms - Math.max(0, now - Date.parse(clock.utc)));
  const t = Math.round(ms / 1000);
  const hh = Math.floor(t / 3600);
  const mm = Math.floor((t % 3600) / 60);
  const ss = String(t % 60).padStart(2, '0');
  return `${hh ? `${hh}:${String(mm).padStart(2, '0')}` : mm}:${ss} left`;
}

function livePage(now) {
  const s = live.summary;
  if (!s) return '<div class="message">The running order appears here during a session.</div>';

  const label = (s.session || '') + (s.kind === 'qualifying' && s.part ? ` · Q${s.part}` : '');
  const progress = s.lap ? `Lap ${s.lap}${s.totalLaps ? ` of ${s.totalLaps}` : ''}` : clockText(s.clock, now);
  const race = s.kind === 'race';

  const rows = s.order
    .map((r) => {
      const badge = r.out ? (race ? 'DNF' : 'STOP') : r.inPit ? 'PIT' : r.pitOut ? 'EXIT' : r.knockedOut ? 'KO' : '';
      const interval = r.pos === 1 ? 'Leader' : gapText(r.interval);
      const gap = r.pos === 1 ? '' : gapText(r.gap);
      const flag = r.lastLap.overall ? ' overall' : r.lastLap.pb ? ' pb' : '';
      return `<li class="${r.out || r.knockedOut ? 'dim' : ''}">
        <span class="pos">${r.pos}</span><i class="team" style="--team:${esc(r.color || '#48484a')}"></i><span class="code">${esc(r.code)}</span>
        <span class="t-main">${badge ? `<b class="badge ${badge.toLowerCase()}">${badge}</b>` : esc(interval)}</span>
        <span class="t-gap">${esc(gap)}</span>
        <span class="t-time${flag}">${esc(r.lastLap.value ?? '')}</span>
        <span class="t-time best">${esc(r.best ?? '')}</span>
        ${tyreCell(r.tyre)}
      </li>`;
    })
    .join('');

  const messages = s.messages
    .slice(0, 4)
    .map((m) => `<li><time>${esc(timeFmt.format(Date.parse(m.utc)))}</time><span>${esc(m.message ?? '')}</span></li>`)
    .join('');

  return `<div class="live-top">${esc(label)}${progress ? ` · ${esc(progress)}` : ''}</div>
    <div class="tower">
      <div class="tower-head"><span></span><span></span><span></span><span>Int.</span><span>Gap</span><span>Last lap</span><span>Best lap</span><span>Tyre</span></div>
      <ul>${rows}</ul>
    </div>
    ${messages ? `<h2 class="spaced">Race control</h2><ul class="rc">${messages}</ul>` : ''}`;
}

// The tabs. While a session is on, Live is the first (and where the island opens) until
// 15 minutes after it finishes (Rust decides: `live.main`); after that it moves to the end.
function tabs() {
  const base = [['weekend', 'Weekend'], ['standings', 'Standings'], ['circuit', 'Circuit']];
  if (!live.summary) return base;
  return live.main ? [['live', 'Live'], ...base] : [...base, ['live', 'Live']];
}

function renderTabs() {
  const list = tabs();
  if (!list.some(([id]) => id === page)) page = 'weekend'; // the live data went away
  $('pages').innerHTML = list
    .map(([id, name]) => `<button data-page="${id}" class="${id === page ? 'active' : ''}">${name}</button>`)
    .join('');
  for (const name of ['live', 'weekend', 'standings', 'circuit']) $(`page-${name}`).hidden = name !== page;
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

  renderTabs();
  weekendPage(now); // also keeps the small island's countdown moving
  if (page === 'live') $('page-live').innerHTML = livePage(now);
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
  render();
}

// ---------- events ----------

// The tabs only work while the island is open, and the island is only clickable then.
$('pages').addEventListener('click', (e) => {
  const button = e.target.closest('button[data-page]');
  if (button) {
    pagePicked = true;
    show(button.dataset.page);
  }
});

// Rust watches the mouse and tells us when it is over the island.
listen('hover', async (event) => {
  island.classList.toggle('open', event.payload);
  if (event.payload) {
    pagePicked = false;
    live = await invoke('live_state'); // fresh: "is Live the main tab?" depends on the time
    show(live.summary && live.main ? 'live' : 'weekend'); // opens on Live while a session is on
  }
  fit();
});

// Rust sends the live state about once a second while a session is being followed.
listen('live:state', (event) => {
  const hadLive = !!live.summary;
  live = event.payload;
  // The island was open on the Weekend when the feed arrived: move to Live, unless a tab was picked.
  if (!hadLive && live.summary && live.main && page === 'weekend' && !pagePicked) page = 'live';
  render();
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

invoke('live_state').then((state) => {
  live = state;
  render();
});
loadWeekend();
loadStandings();
setInterval(loadWeekend, 5 * 60 * 1000);
setInterval(loadStandings, 10 * 60 * 1000);
setInterval(render, 1000);
