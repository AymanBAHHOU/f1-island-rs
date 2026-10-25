const $ = (id) => document.getElementById(id);
const esc = (s) =>
  String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

const TEAM_COLORS = {
  red_bull: '#3671C6',
  ferrari: '#E8002D',
  mercedes: '#27F4D2',
  mclaren: '#FF8000',
  aston_martin: '#229971',
  alpine: '#00A1E8',
  williams: '#64C4FF',
  rb: '#6692FF',
  sauber: '#52E252',
  audi: '#F50537',
  haas: '#B6BABD',
  cadillac: '#C8A45A',
};

const SNAPSHOT_EVERY = 5 * 60e3;
const PEEK_MS = 7000;
const HEADS_UP_MS = 5 * 60e3;
// In auto-hide mode, the pointer must rest this long before the island opens,
// so sweeping past it (e.g. to browser tabs) doesn't trigger it.
const REVEAL_DELAY = 200;
const DRAG_THRESHOLD = 4;

const zone = $('zone');
const island = $('island');
const expandedEl = $('expanded');

let settings = { autoHide: false };
let notch = null;
let notchMotion = null;
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
reducedMotion.addEventListener('change', (event) => notchMotion?.reduce(event.matches));
let snap = null;
let liveData = null;
let realSnapshot = null;
let realLiveData = null;
let demo = null;
let idleEventHead = '';
let idleEventEpoch = 0;
let idleEventKey = '';
let idleTickerAnimation = null;
let hovering = false;
// pinned: sticky (click the island, or "Keep open" in its menu).
// summoned: temporary (hotkey / tray); ends when the mouse leaves the island.
let pinned = new URLSearchParams(location.search).has('pinned');
let summoned = false;
let summonSeen = false; // the mouse has been on the island since it was summoned
let peekUntil = 0;
// Double expanded (analysis) view. While it's open the window is enlarged;
// bigGeom holds that geometry until the window is restored.
let bigOpen = false;
let bigGeom = null;
let bigExitTimer = null;
const BIG_ANIM_MS = 480;
// Pages of the expanded island. Each open starts on the Weekend page.
const PAGES = [
  ['weekend', 'Weekend'],
  ['standings', 'Standings'],
  ['title', 'Title'],
  ['circuit', 'Circuit'],
  ['news', 'News'],
];
// --page=<id> (debug) opens straight on that page.
const debugPage = new URLSearchParams(location.search).get('page');
let page = debugPage || 'weekend';
let wasOpen = !!debugPage;
let pagePicked = false; // a tab was chosen since the island opened
// Live is the first tab and the one the island opens on until 15 minutes after
// the session finishes; then it moves to the end (see livetab.js).
const liveTab = new window.LiveTab();
const liveIsMain = () => liveTab.isMain(!!(liveFeed() || liveOrder()));
const NEWS_TTL = 15 * 60e3;
const news = { items: null, at: 0, loading: false, error: null, open: -1 };
// Track outline for the Circuit page, per weekend (keyed by round).
// retryAt: when a failed or empty load may be tried again (the source can be
// rate-limited or paused during a live session).
const track = { round: null, points: null, loading: false, retryAt: 0 };
const TRACK_RETRY_MS = 2 * 60e3;
// Top three of each finished session this weekend (Weekend page).
const weekendResults = { round: null, list: null, at: 0, loading: false };
const RESULTS_EVERY = 10 * 60e3;
const RESULTS_PENDING_EVERY = 90e3; // while a finished session has no results yet
const RESULTS_DELAY_MIN = 30; // results and analysis appear ~30 min after a session

// ---------- loading animations ----------
// Pages re-render often (live updates), so each animation starts at a phase
// taken from the clock: a re-render carries on instead of restarting.
const phase = (periodMs) => `--phase:-${Date.now() % periodMs}ms`;

// The five start lights come on one by one, then go out.
function startLights(caption) {
  return `<div class="start-lights" style="${phase(4200)}">${'<i></i>'.repeat(5)}</div>${
    caption ? `<div class="loading-cap">${esc(caption)}</div>` : ''
  }`;
}
const MINI_LIGHTS = `<span class="start-lights mini">${'<i></i>'.repeat(5)}</span>`;

// ---------- entrance animations ----------
// Start-lights intro: once per launch, the lights come on and go out while the
// weekend loads underneath.
const INTRO_MS = 2000;
const introStart = Date.now();
const introOn = (now) => now - introStart < INTRO_MS;
const introLights = (now, mini) =>
  `<span class="start-lights intro${mini ? ' mini' : ''}" style="--phase:-${now - introStart}ms">${'<i></i>'.repeat(5)}</span>`;

// Pages slide their rows in the first time they're seen, and again when their
// data changes. Live pages update constantly, so they don't.
const entered = new Map(); // page -> data key last shown with an entrance
function entranceKey(shown) {
  if (shown === 'weekend') return liveOrder() || liveFeed() ? null : String(snap.weekend.round);
  if (shown === 'standings') return snap.standings.length ? snap.standings.map((r) => r.points).join(',') : null;
  if (shown === 'title') return snap.title ? snap.title.drivers.map((d) => d.chance).join(',') : null;
  if (shown === 'circuit') return snap.circuit ? String(snap.weekend.round) : null;
  return null;
}

// A made-up circuit that draws itself while the real map loads.
const LOADING_TRACK =
  'M40,100 L296,100 C336,100 356,84 356,62 C356,38 332,28 312,40 L272,68 C260,76 246,72 242,60 L234,34 C228,18 210,14 194,22 L124,56 C108,64 92,58 88,44 C84,28 64,22 52,34 C36,52 28,90 40,100 Z';
function trackLoading() {
  return `<div class="circuit-map placeholder" style="${phase(3600)}"><svg viewBox="0 0 396 128" aria-hidden="true">
      <path class="tl-ghost" d="${LOADING_TRACK}"/><path class="tl-draw" pathLength="1000" d="${LOADING_TRACK}"/>
    </svg><span>Loading track map…</span></div>`;
}

// Headlines: a two-line title and a small source line each, shimmering.
function headlinesSkeleton(caption, items = 4) {
  const item = (i) =>
    `<div class="sk-news"><span class="sk" style="width:${[92, 84, 90, 78][i % 4]}%"></span><span class="sk" style="width:${[60, 48, 66, 54][i % 4]}%"></span><span class="sk sk-meta" style="width:${[28, 34, 24, 30][i % 4]}%"></span></div>`;
  return `<div class="timing-skeleton" style="${phase(1600)}">${Array.from({ length: items }, (_, i) => item(i)).join('')}</div><div class="loading-cap">${esc(caption)}</div>`;
}

// Grey rows in the shape of a table, with sector blocks flashing like a timing screen.
function timingSkeleton(caption, rows = 4) {
  const row = (i) =>
    `<div class="sk-row"><span class="sk"></span><i class="sk-bar"></i><span class="sk"></span><span class="sk" style="width:${[70, 55, 62, 48, 58][i % 5]}%"></span><span class="sk sk-end"></span></div>`;
  return `<div class="timing-skeleton" style="${phase(1600)}">${Array.from({ length: rows }, (_, i) => row(i)).join('')}</div><div class="loading-cap">${esc(caption)}</div>`;
}
let lastStateKey = null;
let headsUpFor = null;
// Track status codes from live timing that stay on the pill while in force.
const TRACK_FLAGS = { 2: ['yellow', 'YEL'], 4: ['sc', 'SC'], 5: ['red', 'RED'], 6: ['vsc', 'VSC'], 7: ['vsc', 'VSC'] };
// Alerts pop out of the island one at a time.
const alertQueue = [];
let currentAlert = null;
const MAX_QUEUE = 8;
const FLAG_ICON =
  '<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path d="M6 21V4m0 1h11l-2.5 4L17 13H6" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg>';

// ---------- formatting ----------

const pad = (n) => String(n).padStart(2, '0');

function fmtCountdown(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (d > 0) return `${d}d ${pad(h)}h`;
  if (h > 0) return `${h}:${pad(m)}:${pad(sec)}`;
  return `${pad(m)}:${pad(sec)}`;
}

const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: 'short' });
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
const fmtWhen = (iso) => `${dayFmt.format(new Date(iso))} ${timeFmt.format(new Date(iso))}`;

const shortName = (name) => name.replace(/Grand Prix/i, 'GP');

// ---------- state ----------

// The session live timing says is running, which wins over the schedule:
// delays and red flags push sessions past their scheduled end. "Inactive"
// only counts once the scheduled start has passed (a delayed start).
function feedSession(w, now) {
  if (!liveData?.session) return null;
  const s = w.sessions.find((x) => x.name === liveData.session);
  if (!s) return null;
  if (liveData.status === 'Started' || liveData.status === 'Aborted') return s;
  if (liveData.status === 'Inactive' && now >= Date.parse(s.start)) return s;
  return null;
}

function computeState(now) {
  const w = snap?.weekend;
  if (!w) return { kind: snap?.error ? 'error' : 'none' };
  const fromFeed = feedSession(w, now);
  if (fromFeed) return { kind: 'live', session: fromFeed };
  const live = w.sessions.find((s) => now >= Date.parse(s.start) && now < Date.parse(s.end));
  if (live) return { kind: 'live', session: live };
  const next = w.sessions.find((s) => Date.parse(s.start) > now);
  if (next) return { kind: 'next', session: next };
  return { kind: 'finished' };
}

const stateKey = (st) => `${st.kind}:${st.session?.start ?? ''}`;

function sessionStatus(s, now) {
  const w = snap?.weekend;
  const fromFeed = w && feedSession(w, now);
  if (fromFeed) {
    if (fromFeed.start === s.start) return 'live';
    return Date.parse(s.start) < Date.parse(fromFeed.start) ? 'done' : 'upcoming';
  }
  if (now >= Date.parse(s.end)) return 'done';
  if (now >= Date.parse(s.start)) return 'live';
  return 'upcoming';
}

// ---------- rendering ----------

// Race control quotes times in track-local time ("WILL START AT 17:15").
// Convert HH:MM on the message's local day to a UTC timestamp.
function trackTimeToUtc(hh, mm, refMs, gmtOffset) {
  const sign = String(gmtOffset || '').startsWith('-') ? -1 : 1;
  const [oh = 0, om = 0] = String(gmtOffset || '0:0').replace('-', '').split(':').map(Number);
  const offset = sign * (oh * 60 + om) * 60e3;
  const local = new Date(refMs + offset);
  let t = Date.UTC(local.getUTCFullYear(), local.getUTCMonth(), local.getUTCDate(), hh, mm) - offset;
  if (t < refMs - 12 * 3600e3) t += 86400e3; // "00:15" announced late in the evening
  return t;
}

// A new start/restart time announced by race control, if any is still ahead.
function announcedStart(now) {
  if (!liveData?.messages) return null;
  for (const m of liveData.messages) {
    const match = /\b(START|RESUME|RESTART|RESUMPTION)\b.*?\bAT (\d{1,2})[:.](\d{2})\b/i.exec(m.message);
    if (!match) continue;
    const target = trackTimeToUtc(Number(match[2]), Number(match[3]), Date.parse(m.utc), liveData.gmtOffset);
    return target > now - 10 * 60e3 ? { target, message: m.message } : null;
  }
  return null;
}

const FLAG_TEXT = { red: 'Red flag', sc: 'Safety Car', vsc: 'Virtual Safety Car', yellow: 'Yellow flag' };

// Sectors currently under yellow, from the latest "… IN TRACK SECTOR n" messages.
function yellowSectors() {
  const bySector = new Map();
  for (const m of [...(liveData?.messages || [])].reverse()) {
    const match = /(DOUBLE YELLOW|YELLOW|CLEAR) IN TRACK SECTOR (\d+)/i.exec(m.message);
    if (match) bySector.set(Number(match[2]), match[1].toUpperCase());
  }
  return [...bySector]
    .filter(([, f]) => f !== 'CLEAR')
    .sort((a, b) => a[0] - b[0])
    .map(([s, f]) => (f === 'DOUBLE YELLOW' ? `${s} (double)` : String(s)));
}

// The flag in force, described for the open island: { level, tile, title, sub }.
function flagStatus(st) {
  const flag = activeFlag(st);
  if (!flag) return null;
  const [level, tile] = flag;
  const title = liveData.track.status === '7' ? 'VSC ending' : FLAG_TEXT[level];
  const parts = [];
  if (level === 'yellow') {
    const sectors = yellowSectors();
    if (sectors.length) parts.push(`${sectors.length > 1 ? 'Sectors' : 'Sector'} ${sectors.join(', ')}`);
  }
  if (liveData.lap) parts.push(`Lap ${liveData.lap}${liveData.totalLaps ? `/${liveData.totalLaps}` : ''}`);
  return { level, tile, title, sub: parts.join(' · ') || 'In force now' };
}

// Flag badge in the big (analysis) view's header.
function updateBigFlag(st) {
  const el = $('bigFlag');
  if (!el) return;
  const f = flagStatus(st);
  el.hidden = !f;
  if (f) {
    el.className = `big-flag ${f.level}`;
    el.innerHTML = `<span class="flag-tag ${f.level}">${esc(f.tile)}</span>${esc(f.title)}`;
  }
}

// Scheduled time has passed but live timing says the session hasn't begun
// (delayed start). Only trusted when the feed is about this session.
// Not started yet. Between qualifying segments the feed also says Inactive (with
// the next segment, 2 or 3, already set): that's a break, not a delayed start.
function notStarted(st) {
  return st.kind === 'live' && liveData?.session === st.session.name && liveData.status === 'Inactive' && !(liveData.part > 1);
}

// "Qualifying · Q2" while a qualifying segment is on; just the name otherwise.
const liveSessionLabel = () => (liveData?.session || '') + (liveData?.kind === 'qualifying' && liveData.part ? ` · Q${liveData.part}` : '');

// The flag currently in force (from live timing), while a session runs.
function activeFlag(st) {
  if (st.kind !== 'live' || !liveData?.track) return null;
  if (['Finished', 'Finalised', 'Ends'].includes(liveData.status)) return null;
  return TRACK_FLAGS[liveData.track.status] || null;
}

function renderCompact(st, now) {
  const w = snap?.weekend;
  $('cDot').classList.toggle('live', st.kind === 'live');
  $('cLabel').textContent = snap?.demo ? 'DEMO' : w ? shortName(w.name) : 'Formula 1';
  $('cLabel').classList.toggle('demo-label', !!snap?.demo);
  renderNotchIdle(st, now);

  const right = $('cRight');
  if (introOn(now) && !demo) {
    if (!right.querySelector('.intro')) right.innerHTML = introLights(now, true);
    return;
  }
  switch (st.kind) {
    case 'live': {
      if (notStarted(st)) {
        const flag = activeFlag(st);
        const ann = announcedStart(now);
        right.innerHTML =
          (flag ? `<span class="flag-tag ${flag[0]}">${flag[1]}</span>` : '') +
          `${esc(st.session.short)} · <b>${ann ? `Start ${esc(timeFmt.format(new Date(ann.target)))}` : 'Delayed'}</b>`;
        break;
      }
      const leader = liveData?.top?.[0];
      const flag = activeFlag(st);
      const lap = liveData?.lap ? ` · L${liveData.lap}${liveData.totalLaps ? `/${liveData.totalLaps}` : ''}` : '';
      right.innerHTML =
        (flag ? `<span class="flag-tag ${flag[0]}">${flag[1]}</span>` : '<span class="live-tag">LIVE</span> ') +
        esc(st.session.short) +
        lap +
        (leader ? ` · <b>P1 ${esc(leader.code)}</b>` : '');
      break;
    }
    case 'next':
      right.innerHTML = `${esc(st.session.short)} <b>${fmtCountdown(Date.parse(st.session.start) - now)}</b>`;
      break;
    case 'finished': {
      const winner = snap.lastRace?.round === w.round ? snap.lastRace.podium[0] : null;
      right.innerHTML = winner ? `🏁 <b>${esc(winner.code)}</b>` : 'Chequered flag';
      break;
    }
    case 'error':
      right.textContent = 'Offline';
      break;
    default:
      if (snap) right.textContent = 'Season complete';
      // Set once, so the lights keep running (this runs every second).
      else if (!right.querySelector('.start-lights')) right.innerHTML = MINI_LIGHTS;
  }
}

function renderNotchIdle(state, now) {
  const head = window.NotchIdle.events(liveData)[0];
  const headKey = head ? `${head.utc || ''}:${head.message}` : '';
  if (headKey !== idleEventHead) {
    idleEventHead = headKey;
    idleEventEpoch = now;
  }
  const model = window.NotchIdle.model({ state, snapshot: snap, live: liveData, now,
    eventIndex: Math.floor((now - idleEventEpoch) / 6000) });
  for (const [id, value] of Object.entries({ idleKicker: model.kicker, idleTitle: model.title,
    idleStatus: model.status, idleValue: model.value, idleChip: model.chip, idleDetail: model.detail })) {
    $(id).textContent = value;
  }
  $('idleDot').classList.toggle('live', model.live);
  $('idleDetail').title = model.detail;
  $('notchIdle').setAttribute('aria-label', `${model.kicker}: ${model.title}. ${model.status}: ${model.value}. ${model.detail}`);
  zone.dataset.idleTone = model.tone;
  island.dataset.idleKind = model.kind;
  if (model.eventKey !== idleEventKey && idleEventKey && model.eventKey && !reducedMotion.matches) {
    idleTickerAnimation?.cancel();
    idleTickerAnimation = $('idleDetail').animate([
      { opacity: 0, transform: 'translateY(4px)' }, { opacity: 1, transform: 'translateY(0)' },
    ], { duration: 220, easing: 'cubic-bezier(0.22, 0.8, 0.3, 1)' });
  }
  idleEventKey = model.eventKey;
}

function heroHtml(st, now) {
  if (notStarted(st)) {
    const ann = announcedStart(now);
    if (ann) {
      return `<section class="hero"><div class="hero-label">New start ${esc(timeFmt.format(new Date(ann.target)))}<strong>${esc(
        st.session.name
      )}</strong></div><div class="hero-time" data-target="${ann.target}">${fmtCountdown(ann.target - now)}</div></section>`;
    }
    const since = Date.parse(st.session.start);
    return `<section class="hero"><div class="hero-label">Start delayed<strong>${esc(st.session.name)}</strong></div>
      <div class="hero-time" data-since="${since}"><small>Delayed by</small> ${fmtCountdown(now - since)}</div></section>`;
  }
  if (st.kind === 'live' && liveOrder()) {
    const flag = activeFlag(st);
    return `<section class="hero live slim"><div class="hero-label"><strong>${esc(st.session.name)}</strong><span data-progress>${esc(liveProgress())}</span></div>${
      flag ? `<span class="flag-tag ${flag[0]}">${flag[1]}</span>` : ''
    }</section>`;
  }
  if (st.kind === 'live') {
    const top = liveData?.top?.length
      ? `<div class="live-top">${liveData.top
          .map(
            (d) =>
              `<span><em>P${esc(d.pos)}</em><i class="team" style="--team:${esc(d.color || '')}"></i>${esc(d.code)}</span>`
          )
          .join('')}</div>`
      : `<div class="hero-time" data-tick="elapsed">${fmtCountdown(now - Date.parse(st.session.start))}</div>`;
    const label = liveData?.top?.length
      ? liveData.lap
        ? `Lap ${liveData.lap}${liveData.totalLaps ? ` of ${liveData.totalLaps}` : ''}`
        : 'Live order'
      : 'Running for';
    return `<section class="hero live"><div class="hero-label"><strong>${esc(st.session.name)}</strong>${label}</div>${top}</section>`;
  }
  if (st.kind === 'next') {
    return `<section class="hero"><div class="hero-label">Next up<strong>${esc(st.session.name)}</strong></div>
      <div class="hero-time" data-tick="countdown">${fmtCountdown(Date.parse(st.session.start) - now)}</div></section>`;
  }
  if (st.kind === 'finished') {
    return `<section class="hero"><div class="hero-label">Weekend complete<strong>Chequered flag</strong></div></section>`;
  }
  return '';
}

function rowsHtml(rows, withPoints) {
  return `<ul class="rows">${rows
    .map(
      (r) => `<li><span class="pos">${esc(r.pos)}</span><i class="team" style="--team:${
        esc(r.color || TEAM_COLORS[r.teamId] || '')
      }"></i><span class="code">${esc(r.code)}</span>${withPoints ? `<span class="pts">${esc(r.points)}</span>` : ''}</li>`
    )
    .join('')}</ul>`;
}

function ago(iso) {
  if (!iso) return '';
  const mins = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 60e3));
  if (mins < 60) return `${mins}m ago`;
  if (mins < 48 * 60) return `${Math.round(mins / 60)}h ago`;
  return `${Math.round(mins / 1440)}d ago`;
}

const teamBar = (teamId, color) => `<i class="team" style="--team:${esc(color || TEAM_COLORS[teamId] || '')}"></i>`;

// ---------- live timing tower ----------

const TOWER_TYRES = {
  SOFT: ['S', '#ff3b30'],
  MEDIUM: ['M', '#ffd60a'],
  HARD: ['H', '#f2f2f2'],
  INTERMEDIATE: ['I', '#30d158'],
  WET: ['W', '#0a84ff'],
};
const TOWER_PREVIEW = 10; // rows on the Weekend page; the Live page shows everyone

// A lap or sector time, coloured like the timing screens: purple = fastest
// overall, green = personal best, dimmed = previous lap (not yet replaced).
const timeCell = (t) =>
  `<span class="t-time${t?.overall ? ' overall' : t?.pb ? ' pb' : ''}${t?.previous ? ' prev' : ''}">${esc(t?.value || '')}</span>`;

// Running order from live timing while a session is on.
const liveOrder = () => (liveData?.order?.length ? liveData.order : null);

// Leaders are reported as "LAP 12" (races) or an empty gap (practice).
const gapText = (v) => (!v || /^LAP/i.test(v) ? '' : v);

function towerTyre(t) {
  // Unknown compound (e.g. just out of the garage): show nothing.
  if (!t || !TOWER_TYRES[t.compound]) return '<span class="t-tyre"></span>';
  const [letter, colour] = TOWER_TYRES[t.compound];
  return `<span class="t-tyre" title="${esc(t.compound.toLowerCase())}${t.fresh ? ', new set' : ', used set'}"><i style="--c:${colour}">${letter}</i>${esc(t.laps ?? '')}</span>`;
}

// "1:35.291" -> seconds, for finding the fastest lap in the field.
const lapSeconds = (t) => {
  const m = /^(?:([0-9]+):)?([0-9]+(?:[.][0-9]+)?)$/.exec(String(t ?? '').trim());
  return m ? Number(m[1] || 0) * 60 + Number(m[2]) : null;
};

// A sector time, coloured as on a timing screen: purple = fastest overall,
// green = personal best, yellow = slower; dimmed = still the previous lap's.
const sectorCell = (s) =>
  `<span class="t-time sec${s?.overall ? ' overall' : s?.pb ? ' pb' : ''}${s?.previous ? ' prev' : ''}">${esc(s?.value || '')}</span>`;

// The full running order for the Live page: gaps, last and best lap, sectors
// and tyre in one table. 'compact' (default for previews) is just gaps and tyre.
function towerHtml(rows, kind, cls, view = 'compact') {
  if (view === 'full') {
    const race = kind === 'race';
    const fastest = Math.min(...rows.map((r) => lapSeconds(r.best) ?? Infinity));
    const body = rows
      .map((r) => {
        const badge = r.out ? (race ? 'DNF' : 'STOP') : r.inPit ? 'PIT' : r.pitOut ? 'EXIT' : r.knockedOut ? 'KO' : '';
        const interval = r.pos === 1 ? 'Leader' : gapText(r.interval);
        const gap = r.pos === 1 ? '' : gapText(r.gap);
        const best = lapSeconds(r.best);
        return `<li class="${r.out || r.knockedOut ? 'dim' : ''}">
          <span class="pos">${esc(r.pos)}</span>${teamBar(null, r.color)}<span class="code">${esc(r.code)}</span>
          <span class="t-main">${badge ? `<b class="t-badge ${badge.toLowerCase()}">${badge}</b>` : esc(interval)}</span>
          <span class="t-second">${esc(gap)}</span>
          ${timeCell(r.lastLap)}${[0, 1, 2].map((i) => sectorCell(r.sectors?.[i])).join('')}
          <span class="t-time best${best != null && best === fastest ? ' overall' : ''}">${esc(r.best || '')}</span>
          ${towerTyre(r.tyre)}
        </li>`;
      })
      .join('');
    const head = ['Int.', 'Gap', 'Last lap', 'S1', 'S2', 'S3', 'Best lap', 'Tyre'];
    return `<div class="tower full ${cls}"><div class="tower-head"><span></span><span></span><span></span>${head
      .map((h) => `<span>${h}</span>`)
      .join('')}</div><ul>${body}</ul></div>`;
  }
  const race = kind === 'race';
  const head = race ? ['Interval', 'Gap', 'Tyre', 'Pits'] : ['Best lap', 'Gap', 'Tyre', 'Laps'];
  const body = rows
    .map((r) => {
      const badge = r.out ? (race ? 'DNF' : 'STOP') : r.inPit ? 'PIT' : r.pitOut ? 'EXIT' : r.knockedOut ? 'KO' : '';
      const main = race ? (r.pos === 1 ? 'Leader' : gapText(r.interval)) : r.best || '—';
      const second = r.pos === 1 ? '' : gapText(r.gap);
      const extra = race ? r.pits : r.laps ?? '';
      return `<li class="${r.out || r.knockedOut ? 'dim' : ''}">
        <span class="pos">${esc(r.pos)}</span>${teamBar(null, r.color)}<span class="code">${esc(r.code)}</span>
        <span class="t-main">${badge ? `<b class="t-badge ${badge.toLowerCase()}">${badge}</b>` : esc(main)}</span>
        <span class="t-second">${esc(second)}</span>${towerTyre(r.tyre)}<span class="t-extra">${esc(extra)}</span>
      </li>`;
    })
    .join('');
  return `<div class="tower ${cls}"><div class="tower-head"><span></span><span></span><span></span>${head
    .map((h) => `<span>${h}</span>`)
    .join('')}</div><ul>${body}</ul></div>`;
}

// "Lap 23 of 56" in races, time left otherwise.
function liveProgress() {
  if (liveData?.lap) return `Lap ${liveData.lap}${liveData.totalLaps ? ` of ${liveData.totalLaps}` : ''}`;
  // The feed reports the clock occasionally; count down between reports.
  const ms = window.SessionClock.remainingMs(liveData?.clock);
  return ms ? `${window.SessionClock.formatClock(ms)} left` : '';
}

const fmtLap = (sec) => {
  const m = Math.floor(sec / 60);
  const s = (sec - m * 60).toFixed(3).padStart(6, '0');
  return m ? `${m}:${s}` : s;
};
const fmtGap = (g) => (typeof g === 'number' ? `+${g.toFixed(3)}` : String(g ?? ''));

// A small chip beside the menu button when a newer version exists.
let updateState = { status: 'idle' };
function updateChip() {
  const { status, version } = updateState;
  if (status === 'ready') return '<button class="update-chip" data-act title="Restart F1 Island to install the update">Restart to update</button>';
  if (status === 'available') return `<button class="update-chip" data-act title="Open the download page">v${esc(version)} available</button>`;
  if (status === 'downloading') return '<span class="update-chip busy" title="Downloading the update">Updating…</span>';
  return '';
}
window.f1.updateState().then((s) => {
  updateState = s;
  dirty = true;
});
window.f1.onUpdate((s) => {
  updateState = s;
  dirty = true;
  frame();
});

function loadWeekendResults(round, pending) {
  if (weekendResults.loading || demo) return;
  const every = pending ? RESULTS_PENDING_EVERY : RESULTS_EVERY;
  if (weekendResults.round === round && Date.now() - weekendResults.at < every) return;
  if (weekendResults.round !== round) weekendResults.list = null;
  weekendResults.loading = true;
  window.f1
    .weekendResults()
    .then((list) => {
      weekendResults.list = list;
    })
    .catch(() => {
      weekendResults.list = weekendResults.list || [];
    })
    .finally(() => {
      weekendResults.round = round;
      weekendResults.at = Date.now();
      weekendResults.loading = false;
      dirty = true;
      frame();
    });
}

const resultFor = (s) =>
  weekendResults.list?.find((r) => Math.abs(Date.parse(r.start) - Date.parse(s.start)) < 30 * 60e3);

// A finished session's row: its top three, or when they'll arrive.
function resultCell(s, now) {
  const r = resultFor(s);
  if (r?.top.length) {
    return r.top
      .map((p) => `<span class="pod">${teamBar(null, p.color)}<b>${esc(p.code)}</b>${
        p.time != null ? `<small>${fmtLap(p.time)}</small>` : p.gap != null ? `<small>${esc(fmtGap(p.gap))}</small>` : ''
      }</span>`)
      .join('');
  }
  // Still loading: shimmering placeholders where the three drivers will be.
  if (!weekendResults.list) return `<span class="sk-pods" style="${phase(1600)}"><span class="sk"></span><span class="sk"></span><span class="sk"></span></span>`;
  if (r?.unavailable) return '<span class="s-wait">Results unavailable right now</span>';
  const mins = Math.ceil(RESULTS_DELAY_MIN - (now - Date.parse(s.end)) / 60e3);
  return `<span class="s-wait">${mins > 1 ? `Results and analysis in ~${mins} min` : 'Results and analysis soon'}</span>`;
}

// The weekend as one list: finished sessions show their top three and open
// their analysis; the live or next one is marked.
function scheduleHtml(st, now, w) {
  return `<ul class="schedule">${w.sessions
    .map((s) => {
      const status = sessionStatus(s, now);
      const isNext = st.kind === 'next' && st.session.start === s.start;
      const delayed = status === 'live' && st.session?.start === s.start && notStarted(st);
      const right =
        status === 'live'
          ? `<span class="pill live">${delayed ? 'Delayed' : 'Live'}</span>`
          : isNext
            ? '<span class="pill">Next</span>'
            : status === 'done'
              ? resultCell(s, now)
              : '';
      const open = status === 'done' ? ` data-start="${esc(s.start)}" title="Open ${esc(s.name)} analysis"` : '';
      return `<li class="${status}${isNext ? ' next' : ''}"${open}><span class="s-short">${esc(s.short)}</span><span class="s-when">${esc(
        fmtWhen(s.start)
      )}</span><span class="s-res">${right}</span><span class="s-go">${status === 'done' ? '›' : ''}</span></li>`;
    })
    .join('')}</ul>`;
}

function weekendPage(st, now, w) {
  const done = w.sessions.filter((s) => sessionStatus(s, now) === 'done');
  loadWeekendResults(w.round, done.some((s) => !resultFor(s)?.top.length));
  // Before this weekend's first session: a way into the latest analysis anywhere.
  const footer = done.length
    ? '<div class="schedule-hint">Click a finished session for its analysis</div>'
    : '<button class="analysis-btn">Latest session analysis <span>›</span></button>';

  // During a session the running order takes the stage, with the latest
  // race control message under it.
  const order = st.kind === 'live' && liveOrder();
  if (order) {
    const latest = liveFeed()?.filter((m) => !QUIET_MESSAGE.test(m.message)).slice(0, 1) || [];
    // The slim hero already says what's live; the tower gets the room.
    return `${heroHtml(st, now)}
      <div class="tower-title"><h2>Live order</h2><button class="rc-more" data-page="live">All ${order.length} ›</button></div>
      ${towerHtml(order.slice(0, TOWER_PREVIEW), liveData.kind, 'tower-preview', 'compact')}
      ${latest.length ? `<div class="tower-rc">${rcList(latest, 'rc-mini')}</div>` : ''}`;
  }

  // Race control without timing (e.g. before the order arrives).
  const live = liveFeed() ? rcList(liveFeed().filter((m) => !QUIET_MESSAGE.test(m.message)).slice(0, 3), 'rc-mini') : '';
  const below = live
    ? `<div class="cols rc-cols"><section><h2>Race control <button class="rc-more" data-page="live">All ›</button></h2>${live}</section></div>`
    : '';

  return `${heroHtml(st, now)}${scheduleHtml(st, now, w)}${live ? '' : footer}${below}`;
}

// ---------- race control (live) ----------

// "CLEAR IN TRACK SECTOR 5" and lap-time deletions are noise in the short list.
const QUIET_MESSAGE = /^CLEAR IN TRACK SECTOR|\bDELETED\b/i;

// Race control messages while connected to a session, newest first.
const liveFeed = () => (liveData?.messages?.length ? liveData.messages : null);

function rcClass(m) {
  const f = (m.flag || '').toUpperCase();
  if (f === 'RED') return 'red';
  if (f.includes('YELLOW')) return 'yellow';
  if (f === 'GREEN' || f === 'CLEAR') return 'green';
  if (f === 'CHEQUERED') return 'chequered';
  if (f === 'BLUE') return 'info';
  if (m.category === 'SafetyCar') return 'sc';
  return 'plain';
}

function rcList(messages, cls) {
  return `<ul class="rc-list ${cls}">${messages
    .map(
      (m) => `<li><time>${esc(timeFmt.format(new Date(m.utc)))}</time><i class="rc-dot ${rcClass(m)}"></i><span>${esc(m.message)}</span></li>`
    )
    .join('')}</ul>`;
}

// ---------- title fight ----------

// A contender the simulation never saw win is still "<1%", not 0%.
function fmtChance(c, contender) {
  if (c <= 0) return contender ? '<1%' : '0%';
  if (c < 0.01) return '<1%';
  if (c > 0.995) return '>99%';
  return `${Math.round(c * 100)}%`;
}

function titleRows(rows, label) {
  return `<ul class="title-rows">${rows
    .map(
      (r) => `<li>
        ${teamBar(r.teamId, r.color)}<span class="who">${esc(label(r))}</span>
        <span class="pts">${esc(r.points)}</span>
        <span class="gap">${r.gap ? `−${esc(r.gap)}` : ''}</span>
        <span class="chance-bar"><i style="width:${(r.chance * 100).toFixed(1)}%;--team:${esc(r.color || TEAM_COLORS[r.teamId] || '')}"></i></span>
        <span class="pct">${fmtChance(r.chance, r.contender)}</span>
      </li>`
    )
    .join('')}</ul>`;
}

function clinchLine(t, rows, clinch, noun) {
  const leader = rows[0];
  if (!leader) return '';
  if (clinch?.decided || !(t.races + t.sprints)) return `<p class="title-note"><b>${esc(noun(leader))}</b> ${clinch?.decided ? 'has won the title.' : 'wins the title.'}</p>`;
  if (!clinch) return '';
  return `<p class="title-note"><b>${esc(noun(leader))}</b> can clinch at the <b>${esc(clinch.event.name)}</b> at the earliest: it needs a lead of more than ${esc(clinch.lead)} points after it.</p>`;
}

function titlePage() {
  const t = snap.title;
  if (!t) {
    return snap.extrasPending
      ? `<div class="page-body">${timingSkeleton('Loading the title fight…', 5)}</div>`
      : '<div class="message page-body">Title fight unavailable right now.</div>';
  }
  const left = [t.races && `${t.races} race${t.races > 1 ? 's' : ''}`, t.sprints && `${t.sprints} Sprint${t.sprints > 1 ? 's' : ''}`]
    .filter(Boolean)
    .join(' + ');
  const drivers = t.drivers.filter((d) => d.contender);
  const teams = t.teams.filter((d) => d.contender);
  const firstOut = t.drivers.find((d) => !d.contender);
  return `<div class="page-body title-page">
    <div class="standings-note">${left ? `${esc(left)} left · ${t.pointsLeft} points available` : 'Season complete'}${
      snap.standingsNote ? ` · ${esc(snap.standingsNote)}` : ''
    }</div>
    <h2>Drivers' title</h2>
    ${titleRows(drivers, (d) => d.code)}
    ${firstOut && left ? `<p class="title-note muted">${esc(firstOut.code)} and those behind can no longer win.</p>` : ''}
    ${clinchLine(t, drivers, t.clinch, (d) => d.code)}
    <h2 class="spaced">Constructors' title</h2>
    ${titleRows(teams, (d) => d.name)}
    ${t.runs ? `<p class="title-note muted small">Chances are an estimate from recent form (${t.runs.toLocaleString()} simulated seasons). Upgrades, penalties and weather aren't known to it.</p>` : ''}
  </div>`;
}

// The Live page: the whole field, then the race control feed, in one
// scrolling list.
function livePage() {
  const order = liveOrder();
  const feed = liveFeed();
  if (!order && !feed) return '<div class="message page-body">The running order and race control appear here during a session.</div>';
  const progress = liveProgress();
  const legend = order
    ? '<p class="title-note muted small">Lap and sector times: purple fastest overall · green personal best · yellow slower · dimmed: still the previous lap.</p>'
    : '';
  const top = `<div class="live-top-row">${progress ? `<div class="standings-note">${esc(liveSessionLabel())} · <span data-progress>${esc(progress)}</span></div>` : '<span></span>'}</div>`;
  // Wide: the timing table beside the race control feed, each scrolling on its own.
  if (order && !notch) {
    return `<div class="page-body live-page live-wide">
      <div class="live-main">${top}${towerHtml(order, liveData.kind, 'tower-full', 'full')}${legend}</div>
      <div class="live-side"><h2>Race control</h2>${feed ? rcList(feed, 'rc-in-live') : '<p class="muted small">Messages from race control appear here.</p>'}</div>
    </div>`;
  }
  return `<div class="page-body live-page">
    <div class="live-top-row">${progress ? `<div class="standings-note">${esc(liveSessionLabel())} · <span data-progress>${esc(progress)}</span></div>` : '<span></span>'}</div>
    ${order ? towerHtml(order, liveData.kind, 'tower-full', 'full') : ''}${
      order ? '<p class="title-note muted small">Lap and sector times: purple fastest overall · green personal best · yellow slower · dimmed: still the previous lap.</p>' : ''
    }
    ${feed ? `<h2 class="spaced">Race control</h2>${rcList(feed, 'rc-in-live')}` : ''}
  </div>`;
}

function standingsPage() {
  const leader = Number(snap.standings[0]?.points) || 0;
  const drivers = `<ul class="rows">${snap.standings
    .map(
      (r) => `<li><span class="pos">${esc(r.pos)}</span>${teamBar(r.teamId, r.color)}<span class="code">${esc(r.code)}</span>
        <span class="pts">${esc(r.points)}</span><span class="gap">${
          Number(r.pos) === 1 ? '' : `−${leader - Number(r.points)}`
        }</span></li>`
    )
    .join('')}</ul>`;
  const teams = `<ul class="rows">${snap.constructors
    .slice(0, 10)
    .map(
      (r) => `<li><span class="pos">${esc(r.pos)}</span>${teamBar(r.teamId, r.color)}<span class="name">${esc(r.name)}</span>
        <span class="pts">${esc(r.points)}</span></li>`
    )
    .join('')}</ul>`;
  if (!snap.standings.length && !snap.constructors.length) return '<div class="message">Standings unavailable.</div>';
  const note = snap.standingsNote ? `<div class="standings-note">${esc(snap.standingsNote)}</div>` : '';
  // The last race's podium explains the latest points.
  const last = snap.lastRace;
  const lastRace = last
    ? `<div class="last-race"><h2>${esc(shortName(last.name))}</h2><div class="last-podium">${last.podium
        .map((r) => `<span class="pod"><small>${esc(r.pos)}</small>${teamBar(r.teamId, r.color)}<b>${esc(r.code)}</b></span>`)
        .join('')}</div></div>`
    : '';
  return `<div class="page-body">${note}${lastRace}<div class="cols"><section><h2>Drivers</h2>${drivers}</section><section><h2>Constructors</h2>${teams}</section></div></div>`;
}

function loadTrackMap(round) {
  if (track.loading) return;
  if (track.round === round && (track.points || Date.now() < track.retryAt)) return;
  if (track.round !== round) track.points = null;
  track.loading = true;
  window.f1
    .trackMap()
    .then((m) => {
      track.points = m?.points || null;
    })
    .catch(() => {
      track.points = null;
    })
    .finally(() => {
      track.round = round;
      track.retryAt = Date.now() + TRACK_RETRY_MS;
      track.loading = false;
      dirty = true;
      frame();
    });
}

// A race's total time ("1:34:00.161") written so it can't pass for a lap time.
function raceTime(t) {
  const parts = String(t ?? '').split(':');
  if (parts.length < 2) return t ?? '';
  const sec = parts.pop();
  const min = parts.pop();
  const hours = parts.pop();
  return `${hours ? `${Number(hours)}h ` : ''}${min.padStart(2, '0')}m ${sec}s`;
}

function circuitPage() {
  const c = snap.circuit;
  loadTrackMap(snap.weekend.round);
  // While the map loads, hold its space so the page doesn't jump when it arrives.
  // The first time a map is shown, it draws itself in from the start line.
  const enter = track.points && track.shownRound !== snap.weekend.round;
  if (enter) track.shownRound = snap.weekend.round;
  const map = track.points
    ? `<div class="circuit-map${enter ? ' enter' : ''}">${window.TrackMap.outline(track.points, 396, 128, { pad: 8, stroke: 5 }).replace(
        '<path d="',
        '<path class="tm-glow" d="'
      ).replace(/<path d="/, '<path class="tm-line" pathLength="1000" d="')}</div>`
    : track.loading
      ? trackLoading()
      : '';
  if (!c) {
    const body = snap.extrasPending
      ? timingSkeleton('Loading circuit history…', 4)
      : '<div class="message">Circuit history unavailable right now.</div>';
    return `<div class="page-body">${map}${body}</div>`;
  }
  if (!c.last && !c.winners.length) {
    return `<div class="page-body">${map}<div class="message">No previous races at this circuit.</div></div>`;
  }
  const fact = (label, entry, value) =>
    entry ? `<li><span class="label">${label}</span>${teamBar(entry.teamId)}<span class="code">${esc(entry.code)}</span><span class="val">${value}</span></li>` : '';
  const l = c.last;
  const lastHtml = l
    ? `<div class="cols">
        <section><h2>Last time here · ${esc(l.season)}</h2><ul class="facts">
          ${fact('Pole lap', l.pole, esc(l.pole?.time ?? ''))}
          ${fact('Race time', l.podium[0], esc(raceTime(l.winnerTime)))}
          ${fact('Fastest lap', l.fastestLap, l.fastestLap ? `${esc(l.fastestLap.time)} <small>L${esc(l.fastestLap.lap)}</small>` : '')}
          <li><span class="label">Race laps</span><span class="val">${esc(l.laps)}</span></li>
        </ul></section>
        <section><h2>Podium</h2>${rowsHtml(l.podium, false)}</section>
      </div>`
    : '';
  const winners = c.winners.length
    ? `<h2 class="spaced">Recent winners</h2><div class="winners">${c.winners
        .map((x) => `<span><small>${esc(x.season)}</small>${teamBar(x.teamId)}<b>${esc(x.code)}</b></span>`)
        .join('')}</div>`
    : '';
  return `<div class="page-body">${map}${lastHtml}${winners}</div>`;
}

function loadNews() {
  if (news.loading) return;
  news.loading = true;
  window.f1
    .news()
    .then((items) => {
      news.items = items;
      news.error = null;
      news.at = Date.now();
    })
    .catch((err) => {
      news.error = err.message;
    })
    .finally(() => {
      news.loading = false;
      dirty = true;
      frame();
    });
}

function newsPage() {
  if (!news.items || Date.now() - news.at > NEWS_TTL) loadNews();
  if (!news.items) {
    return news.error
      ? `<div class="message page-body">${esc(news.error)}</div>`
      : `<div class="page-body">${headlinesSkeleton('Loading headlines…')}</div>`;
  }
  return `<ul class="news-list page-body">${news.items
    .map((n, i) => {
      const open = news.open === i;
      return `<li class="${open ? 'open' : ''}">
        <button class="news-head" data-news="${i}">
          <div class="n-meta"><b>${esc(n.source)}</b> · ${esc(ago(n.date))}</div>
          <div class="n-title">${esc(n.title)}</div>
        </button>
        ${
          open
            ? `${n.summary ? `<p class="n-summary">${esc(n.summary)}</p>` : ''}<button class="news-open" data-news-open="${i}">Read on ${esc(
                n.source
              )} ↗</button>`
            : ''
        }
      </li>`;
    })
    .join('')}</ul>`;
}

const flagTile = (a, cls = 'flag-tile') =>
  `<span class="${cls} ${esc(a.level)}">${a.tile ? esc(a.tile) : a.level === 'chequered' ? '' : FLAG_ICON}</span>`;

const alertBanner = (a) =>
  `<div class="alert-banner ${esc(a.level)}">${flagTile(a, 'flag-tile small')}<div><b>${esc(a.title)}</b><span>${esc(a.sub)}</span></div></div>`;

function renderAlertView() {
  const a = currentAlert;
  if (!a) return;
  $('alertView').innerHTML = `${flagTile(a)}<div class="a-text"><div class="a-title">${esc(a.title)}</div><div class="a-sub">${esc(
    a.sub
  )}</div></div>`;
  island.dataset.level = a.level;
}

function queueAlert(a) {
  alertQueue.push(a);
  // Under a flood of messages, keep the important ones.
  while (alertQueue.length > MAX_QUEUE) {
    const i = alertQueue.findIndex((x) => x.level === 'info');
    alertQueue.splice(i >= 0 ? i : 0, 1);
  }
  if (!currentAlert) nextAlert();
}

function nextAlert() {
  currentAlert = alertQueue.shift() || null;
  dirty = true;
  if (!currentAlert) {
    delete island.dataset.level;
    frame();
    return;
  }
  renderAlertView();
  frame();
  setTimeout(() => {
    currentAlert = null;
    dirty = true;
    frame();
    // Let the island settle between alerts.
    setTimeout(nextAlert, 450);
  }, currentAlert.ms || 6000);
}

function renderExpanded(st, now) {
  const w = snap?.weekend;
  if (!w && !snap && st.kind !== 'error') {
    expandedEl.innerHTML = `<div class="eyebrow">Formula 1</div><div class="loading-block">${startLights('Getting the weekend ready…')}</div>`;
    return;
  }
  if (!w) {
    expandedEl.innerHTML = `<div class="eyebrow">Formula 1</div><h1>${
      st.kind === 'error' ? 'Can’t reach F1 data' : snap ? 'Season complete' : 'Loading…'
    }</h1>${snap ? `<div class="message">${esc(snap.error || 'See you next season.')}</div>` : ''}`;
    return;
  }

  // A Race control page joins the switcher while connected to a session.
  // A Live page (running order + race control) joins the switcher during a session.
  const isLive = !!(liveFeed() || liveOrder());
  const pages = isLive ? (liveIsMain() ? [['live', 'Live'], ...PAGES] : [...PAGES, ['live', 'Live']]) : PAGES;
  if (page === 'control') page = 'live'; // old page name (debug flag)
  // Keep a chosen Live page across feed hiccups; show Weekend meanwhile.
  const shown = page === 'live' && !isLive ? 'weekend' : page;
  const nav = `<nav class="pages">${pages
    .map(([id, label]) => `<button data-page="${id}" class="${shown === id ? 'active' : ''}">${label}</button>`)
    .join('')}</nav>`;

  const body =
    shown === 'live'
      ? livePage()
      : page === 'standings'
      ? standingsPage()
      : page === 'title'
      ? titlePage()
      : page === 'circuit'
        ? circuitPage()
        : page === 'news'
          ? newsPage()
          : weekendPage(st, now, w);

  // Keep scroll positions (news list, Live page and its two columns) across re-renders.
  const SCROLLERS = '.news-list, .rc-full, .live-page, .live-main, .live-side';
  const scrollTops = [...expandedEl.querySelectorAll(SCROLLERS)].map((el) => el.scrollTop);

  // Only when it can be seen: open, and not behind the analysis view.
  const key = wasOpen && !bigOpen ? entranceKey(shown) : null;
  const enter = key != null && entered.get(shown) !== key;
  if (enter) entered.set(shown, key);
  expandedEl.classList.toggle('page-enter', enter && !introOn(now));
  const intro = introOn(now) && !demo ? `<div class="intro-overlay" style="--phase:-${now - introStart}ms">${introLights(now, false)}<span>Lights out</span></div>` : '';

  expandedEl.innerHTML = `${intro}
    <button class="menu-btn" title="Settings">⋯</button>${updateChip()}
    <div class="eyebrow">Round ${esc(w.round)} · ${esc(w.season)}</div>
    <h1>${esc(w.name)}</h1>
    <div class="sub">${esc(w.circuit)} · ${esc(w.country)}</div>
    ${currentAlert ? alertBanner(currentAlert) : flagStatus(st) ? alertBanner(flagStatus(st)) : ''}
    ${nav}
    ${body}`;

  expandedEl.querySelectorAll(SCROLLERS).forEach((el, i) => {
    if (scrollTops[i]) el.scrollTop = scrollTops[i];
  });
}

function tickExpanded(st, now) {
  for (const el of expandedEl.querySelectorAll('[data-progress]')) el.textContent = liveProgress();
  // Countdown to an announced time, or time elapsed since one.
  const target = expandedEl.querySelector('[data-target]');
  if (target) target.textContent = fmtCountdown(Number(target.dataset.target) - now);
  const since = expandedEl.querySelector('[data-since]');
  if (since) since.innerHTML = `<small>Delayed by</small> ${fmtCountdown(now - Number(since.dataset.since))}`;

  const el = expandedEl.querySelector('[data-tick]');
  if (!el || !st.session) return;
  el.textContent =
    el.dataset.tick === 'countdown'
      ? fmtCountdown(Date.parse(st.session.start) - now)
      : fmtCountdown(now - Date.parse(st.session.start));
}

function layout(now) {
  document.body.classList.toggle('docked', !!settings.docked && !notch);
  const open = pinned || summoned || hovering || (!notch && now < peekUntil);
  if (open && !wasOpen) {
    entered.clear(); // so does each time the island opens
    // The island opens on Live while a session is on, otherwise on the Weekend.
    const home = liveIsMain() ? 'live' : 'weekend';
    if (page !== home) {
      page = home;
      news.open = -1;
    }
    pagePicked = false;
    wasOpen = true; // renderExpanded plays entrances only when open
    renderExpanded(computeState(now), now);
  }
  wasOpen = open;
  if (!open) bigOpen = false; // collapsing closes the analysis view too
  const big = open && bigOpen;
  const alerting = !notch && !open && !!currentAlert;
  const hidden = !open && !alerting && settings.autoHide && !demo;
  const stretched = !!notch && !open && !hidden;
  island.classList.toggle('alert', alerting);
  const flag = activeFlag(computeState(now));
  // Keep the entire camera silhouette black, including expanded flag views.
  // Flag colours remain in the content badges; only floating mode uses rings.
  if (flag && !notch) island.dataset.track = flag[0];
  else delete island.dataset.track;
  island.classList.toggle('big', big);
  // The Live tab opens wide (table beside the race control feed).
  island.classList.toggle('wide', open && !big && !notch && page === 'live' && !!liveOrder());
  island.classList.toggle('expanded', open && !big);
  island.classList.toggle('hidden', hidden);
  island.classList.toggle('pinned', pinned);
  island.classList.toggle('stretched', stretched);
  zone.classList.toggle('autohide', hidden);
  if (notchMotion) {
    notchMotion.move({
      width: big ? bigGeom.width - 40 : open ? 440 : alerting ? 410 : hidden ? notch.width - 4 : Math.min(notch.width + 264, 480),
      height: big ? bigGeom.height - bigGeom.edge - 16 : open ? expandedEl.offsetHeight
        : alerting ? notch.height + 76 : hidden ? notch.height - 1 : notch.height + 24,
      radius: big || open ? 28 : alerting ? 24 : hidden ? 10 : 16,
      shoulder: big || open ? 20 : alerting ? 16 : hidden ? 0 : 12,
    });
  } else {
    island.style.height = big ? `${bigGeom.height - bigGeom.edge - 16}px` : open ? `${expandedEl.offsetHeight}px` : '';
  }

  // Leaving big mode: shrink back towards the island's spot, then restore the window.
  if (!big && bigGeom) {
    zone.style.left = `${bigGeom.anchorX}px`;
    if (!bigExitTimer) bigExitTimer = setTimeout(finishBigExit, BIG_ANIM_MS);
  }
}

// Jump without animating (used when the window is resized under the island).
function placeZone(left) {
  zone.style.transition = 'none';
  zone.style.left = left;
  void zone.offsetWidth;
  zone.style.transition = '';
}

async function openBig(start) {
  clearTimeout(bigExitTimer);
  bigExitTimer = null;
  if (!bigGeom) {
    bigGeom = await window.f1.setBig(true);
    // The window grew around the island: keep it in place, then grow to the centre.
    placeZone(`${bigGeom.anchorX}px`);
    island.style.setProperty('--big-w', `${bigGeom.width - 40}px`);
  }
  bigOpen = true;
  zone.style.left = '50%';
  layout(Date.now());
  window.AnalysisView.show(start);
}

// Back from the analysis view: the island shrinks away from the cursor, so
// keep it open (summoned) until the mouse comes back onto it and leaves again.
function closeBig() {
  bigOpen = false;
  summon();
  clearTimeout(leaveTimer);
  hovering = false;
  window.f1.setInteractive(false);
  window.f1.rehover(); // re-sends "enter" if the cursor is still on the island
  layout(Date.now());
}

function summon() {
  summoned = true;
  summonSeen = false;
}

async function finishBigExit() {
  bigExitTimer = null;
  if (bigOpen || !bigGeom) return;
  bigGeom = null;
  await window.f1.setBig(false);
  placeZone('');
  reportRect();
}

let dirty = true;
let lastWasLive = false;
function frame() {
  const now = Date.now();
  const st = computeState(now);
  const key = stateKey(st);

  if (key !== lastStateKey) {
    // A session just went live: peek so the change is noticed.
    if (lastStateKey && st.kind === 'live') peekUntil = now + PEEK_MS;
    lastStateKey = key;
    dirty = true;
    // Results/standings change after a session ends.
    if (lastWasLive && st.kind !== 'live') setTimeout(refreshSnapshot, 60e3);
    lastWasLive = st.kind === 'live';
  }

  // Heads-up peek shortly before a session starts.
  if (st.kind === 'next' && Date.parse(st.session.start) - now <= HEADS_UP_MS && headsUpFor !== st.session.start) {
    headsUpFor = st.session.start;
    peekUntil = now + PEEK_MS;
  }

  renderCompact(st, now);
  updateBigFlag(st);
  if (dirty) {
    renderExpanded(st, now);
    dirty = false;
  } else {
    tickExpanded(st, now);
  }
  layout(now);
}

// ---------- data ----------

async function refreshSnapshot() {
  if (demo) {
    snap = demo.snapshot;
    dirty = true;
    frame();
    return;
  }
  try {
    // First load: show the weekend as soon as it's known, then fill in the
    // title fight and circuit history.
    if (!snap?.weekend) {
      const core = await window.f1.snapshot({ extras: false });
      if (!demo && core.weekend) {
        snap = realSnapshot = core;
        dirty = true;
        frame();
      }
    }
    realSnapshot = await window.f1.snapshot();
  } catch (err) {
    // Keep what's already shown (e.g. the first, quick snapshot) over an error.
    realSnapshot = snap?.weekend && !demo ? snap : { weekend: null, lastRace: null, standings: [], error: String(err.message || err) };
  }
  if (demo) return; // an in-flight real response must never overwrite a preview
  snap = realSnapshot;
  // Fetch the Circuit page's map in the background, so it's ready when opened.
  if (snap.weekend) loadTrackMap(snap.weekend.round);
  if (snap.weekend) loadWeekendResults(snap.weekend.round, false);
  dirty = true;
  frame();
}

window.f1.onLiveState((s) => {
  realLiveData = s;
  if (demo) return;
  const wasLive = !!(liveFeed() || liveOrder());
  liveData = s;
  const weekend = snap?.weekend;
  liveTab.update(s, Date.now(), Date.parse(weekend?.sessions.find((x) => x.name === s?.session)?.end) || null);
  // The island was already open on the Weekend when the feed arrived (start-up,
  // or a session just began): move to Live, unless a tab was picked meanwhile.
  if (!wasLive && liveIsMain() && page === 'weekend' && !pagePicked) {
    page = 'live';
    entered.clear();
  }
  dirty = true;
});
window.f1.onAlert((alert) => { if (!demo) queueAlert(alert); });

// ---------- interaction ----------

let leaveTimer = null;
let revealTimer = null;
let press = null;
let dragging = false;

function reveal() {
  hovering = true;
  summonSeen = true;
  window.f1.setInteractive(true);
  layout(Date.now());
}

// Hover (reported by the main process). Until the window turns interactive it
// is click-through, so a delayed reveal in auto-hide mode never steals clicks
// from the app underneath.
function onEnter() {
  clearTimeout(leaveTimer);
  clearTimeout(revealTimer);
  if (notch && !hovering) revealTimer = setTimeout(reveal, 90);
  else if (hovering || !island.classList.contains('hidden')) reveal();
  else revealTimer = setTimeout(reveal, REVEAL_DELAY);
}
function onLeave() {
  if (dragging) return;
  clearTimeout(revealTimer);
  window.f1.setInteractive(false);
  leaveTimer = setTimeout(() => {
    hovering = false;
    if (summonSeen) summoned = false;
    layout(Date.now());
  }, notch ? 350 : 250);
}
window.f1.onHover((inside) => (inside ? onEnter() : onLeave()));

// Keep the main process's copy of the hover area in sync as the island resizes.
function reportRect() {
  const r = zone.getBoundingClientRect();
  window.f1.setRect({ x: r.x, y: r.y, width: r.width, height: r.height });
}
new ResizeObserver(reportRect).observe(zone);
zone.addEventListener('transitionend', reportRect); // position changes don't resize
reportRect();

// Press-and-drag moves the island; a press without movement toggles pinning.
zone.addEventListener('pointerdown', (e) => {
  // Clickable things keep their click: capturing the pointer would send it to the island.
  if (e.button !== 0 || !hovering || e.target.closest('button, [data-start], .news-list, .rc-full, .live-page')) return;
  // In the analysis view only the header drags; the rest is for reading/clicking.
  if (bigOpen && !e.target.closest('.big-head')) return;
  press = { x: e.screenX, y: e.screenY };
  zone.setPointerCapture(e.pointerId);
});
zone.addEventListener('pointermove', (e) => {
  if (!press || dragging) return;
  if (Math.hypot(e.screenX - press.x, e.screenY - press.y) > DRAG_THRESHOLD) {
    dragging = true;
    zone.classList.add('dragging');
    window.f1.dragStart();
  }
});

function release(isClick) {
  if (!press) return;
  const wasDragging = dragging;
  press = null;
  dragging = false;
  zone.classList.remove('dragging');
  if (wasDragging) {
    window.f1.dragEnd();
  } else if (isClick && !bigOpen) {
    setPinned(!pinned);
  }
}
zone.addEventListener('pointerup', () => release(true));
zone.addEventListener('lostpointercapture', () => release(false));

// Settings menu: right-click anywhere on the island, or the ⋯ button.
zone.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  window.f1.openMenu();
});
expandedEl.addEventListener('click', (e) => {
  if (e.target.closest('.menu-btn')) window.f1.openMenu();
  if (e.target.closest('.update-chip[data-act]')) window.f1.updateAct();
  const analysis = e.target.closest('.analysis-btn, .schedule li[data-start]');
  if (analysis) openBig(analysis.dataset.start || undefined);
  const pageBtn = e.target.closest('[data-page]');
  if (pageBtn && pageBtn.dataset.page !== page) {
    page = pageBtn.dataset.page;
    pagePicked = true;
    entered.clear(); // each visit to a page plays its entrance
    news.open = -1;
    dirty = true;
    frame();
  }
  const head = e.target.closest('[data-news]');
  if (head) {
    const i = Number(head.dataset.news);
    news.open = news.open === i ? -1 : i;
    dirty = true;
    frame();
  }
  const read = e.target.closest('[data-news-open]');
  if (read) window.f1.openArticle(news.items[Number(read.dataset.newsOpen)]?.link);
});
$('bigBack').addEventListener('click', closeBig);

function setPinned(on) {
  pinned = on;
  window.f1.reportPinned(pinned);
  layout(Date.now());
}

// Global hotkey / tray click: open temporarily, or close whatever is open.
function toggle() {
  const now = Date.now();
  if (pinned || summoned || (!notch && now < peekUntil)) {
    summoned = false;
    peekUntil = 0;
    if (pinned) setPinned(false);
  } else {
    summon();
  }
  layout(now);
}

window.f1.onToggle(toggle);
// Main moved the island out from under the pointer: keep it open until it's used again.
window.f1.onSummon(() => {
  if (!(pinned || hovering)) summon();
});
// The big island was dragged: the small island's spot moved with it.
window.f1.onBigAnchor((x) => {
  if (bigGeom) bigGeom.anchorX = x;
});
window.f1.onBigGeometry((geometry) => {
  if (!bigGeom) return;
  bigGeom = geometry;
  island.style.setProperty('--big-w', `${geometry.width - 40}px`);
  placeZone(bigOpen ? '50%' : `${geometry.anchorX}px`);
  layout(Date.now());
  reportRect();
});
// Menu / tray "Session analysis": open the island straight into the big view.
window.f1.onBig(() => {
  if (!(pinned || hovering)) summon();
  openBig();
});
window.f1.onSetPinned(setPinned);
window.f1.reportPinned(pinned);
window.f1.onRefresh(refreshSnapshot);
window.f1.onSettings((s) => {
  settings = s;
  layout(Date.now());
});
window.f1.settings().then((s) => {
  settings = s;
  layout(Date.now());
});

function applyNotch(next) {
  notch = next;
  if (!next && notchMotion) {
    notchMotion.stop();
    notchMotion = null;
  }
  document.body.classList.toggle('notched', !!next);
  document.body.style.setProperty('--notch-w', `${next?.width || 0}px`);
  document.body.style.setProperty('--notch-h', `${next?.height || 0}px`);
  document.body.style.setProperty('--idle-w', `${next ? Math.min(next.width + 264, 480) : 0}px`);
  if (next && !notchMotion) {
    // Begin behind the physical cutout, not outside its visible edges.
    notchMotion = window.NotchMotion.create(island, document.querySelector('.notch-surface'), {
      width: next.width - 4, height: next.height - 1, radius: 10, shoulder: 0,
    }, reducedMotion.matches);
  }
  layout(Date.now());
  reportRect();
}
window.f1.onNotch(applyNotch);
window.f1.notch().then(applyNotch);

function applyDemo(next) {
  demo = next;
  snap = next ? next.snapshot : realSnapshot;
  liveData = next ? next.live : realLiveData;
  pinned = summoned = hovering = bigOpen = false;
  peekUntil = 0;
  lastStateKey = null;
  headsUpFor = null;
  idleEventHead = idleEventKey = '';
  alertQueue.length = 0;
  currentAlert = null;
  window.f1.reportPinned(false);
  dirty = true;
  frame();
  if (!next) refreshSnapshot();
}
window.f1.onDemo(applyDemo);
window.f1.demo().then((next) => {
  if (!next) return;
  applyDemo(next);
  // A demo starts collapsed, but the --pinned debug flag still wins at start-up.
  if (new URLSearchParams(location.search).has('pinned')) {
    setPinned(true);
    if (debugPage) {
      page = debugPage;
      dirty = true;
      frame();
    }
  }
});

refreshSnapshot().then(() => {
  const params = new URLSearchParams(location.search);
  if (params.has('big')) openBig(params.get('start') || undefined);
});
setInterval(refreshSnapshot, SNAPSHOT_EVERY);
// Pick up a finished session's results soon after they're published.
setInterval(() => {
  const w = snap?.weekend;
  if (!w) return;
  const now = Date.now();
  loadWeekendResults(w.round, w.sessions.some((s) => sessionStatus(s, now) === 'done' && !resultFor(s)?.top.length));
}, 60e3);
setInterval(frame, 1000);
