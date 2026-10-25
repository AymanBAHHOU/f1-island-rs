// The page side. `withGlobalTauri` in tauri.conf.json puts Tauri's API on
// window.__TAURI__, so no bundler is needed.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const island = $('island');
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

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

let weekend = null; // what Rust gave us: { season, round, name, circuit, country, sessions: [...] }
let problem = null; // the reason it couldn't, if it couldn't

// 1. Ask Rust for the weekend. Rust fetches and caches it; we ask again every few minutes.
async function load() {
  try {
    weekend = await invoke('weekend');
    problem = weekend ? null : 'Season complete';
  } catch (err) {
    problem = String(err);
  }
  render();
}

// 2. Draw it. Runs every second so the countdown moves.
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

  // The next session still to finish: it's live if it has already started.
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

// 3. Rust watches the mouse and tells us when it is over the island.
listen('hover', (event) => island.classList.toggle('open', event.payload));

// 4. Tell Rust where the island is, so it knows what "over the island" means.
//    The island grows and shrinks, so report whenever its zone changes size.
const zone = $('zone');
const report = () => {
  const r = zone.getBoundingClientRect();
  invoke('report_rect', { rect: { x: r.x, y: r.y, width: r.width, height: r.height } });
};
new ResizeObserver(report).observe(zone);
report();

load();
setInterval(load, 5 * 60 * 1000);
setInterval(render, 1000);
