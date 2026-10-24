// The page side of stage 1. `withGlobalTauri` in tauri.conf.json puts Tauri's
// API on window.__TAURI__, so no bundler is needed.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const island = $('island');

const fmt = (secs) => {
  const s = Math.max(0, secs);
  const h = Math.floor(s / 3600);
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, '0');
  const r = String(s % 60).padStart(2, '0');
  return `${h}:${m}:${r}`;
};

// 1. Ask Rust for the data (a Rust function runs and returns a JSON object).
async function start() {
  const status = await invoke('island_status');
  $('title').textContent = status.title;
  $('subtitle').textContent = status.subtitle;
  $('next').textContent = status.next;
  $('cLabel').textContent = 'US GP';

  const t0 = Date.now();
  const tick = () => {
    const left = status.starts_in_secs - Math.floor((Date.now() - t0) / 1000);
    $('count').textContent = fmt(left);
    $('cRight').innerHTML = `${status.next.replace('Practice ', 'FP')} <b>${fmt(left)}</b>`;
  };
  tick();
  setInterval(tick, 1000);
}

// 2. Rust watches the mouse and tells us when it is over the island.
listen('hover', (event) => island.classList.toggle('open', event.payload));

// 3. Tell Rust where the island is, so it knows what "over the island" means.
//    The island grows and shrinks, so report whenever its zone changes size.
const zone = $('zone');
const report = () => {
  const r = zone.getBoundingClientRect();
  invoke('report_rect', { rect: { x: r.x, y: r.y, width: r.width, height: r.height } });
};
new ResizeObserver(report).observe(zone);
report();

start();
