// Session analysis view, shown inside the island in its "double expanded" state.
(() => {
const api = window.f1.analysis;
const $ = (id) => document.getElementById(id);
const esc = (s) =>
  String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

const COMPOUNDS = {
  SOFT: { color: '#ff3b30', label: 'S' },
  MEDIUM: { color: '#ffd60a', label: 'M' },
  HARD: { color: '#f2f2f2', label: 'H' },
  INTERMEDIATE: { color: '#30d158', label: 'I' },
  WET: { color: '#0a84ff', label: 'W' },
  UNKNOWN: { color: '#636366', label: '?' },
};
const COMPOUND_ORDER = ['SOFT', 'MEDIUM', 'HARD', 'INTERMEDIATE', 'WET', 'UNKNOWN'];
const DEFAULT_COMPARE = 6; // drivers compared by default; more lines get unreadable
const MIN_LAPS_CHOICES = [3, 4, 5, 6, 8, 10];

const state = {
  sessions: [],
  key: null,
  data: null,
  tab: new URLSearchParams(location.search).get('tab') || 'summary',
  minLaps: 4, // shortest run counted as a long run (user-adjustable)
  selected: null, // Set of driver numbers to compare; null = fastest DEFAULT_COMPARE
  cmp: null, // Compare tab: { a: { driver, lap }, b: { driver, lap } }
  tele: new Map(), // telemetry by "session/driver/lap": data | 'loading' | { error }
  locs: new Map(), // lap positions by "session/driver/lap": data | 'loading' | { error }
  replay: null, // Replay tab state (see replay section)
  loadToken: 0,
};

// ---------- formatting ----------

function fmtLap(s) {
  if (s == null || !Number.isFinite(s)) return '—';
  const m = Math.floor(s / 60);
  const rest = s - m * 60;
  return m ? `${m}:${rest.toFixed(3).padStart(6, '0')}` : rest.toFixed(3);
}

function fmtTotal(s) {
  if (s == null || !Number.isFinite(s)) return '—';
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = (s % 60).toFixed(3).padStart(6, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

function fmtGap(g) {
  if (g == null) return '';
  if (typeof g === 'string') return g;
  return g === 0 ? '—' : `+${g.toFixed(3)}`;
}

const fmtSigned = (v, digits = 3) => `${v >= 0 ? '+' : '−'}${Math.abs(v).toFixed(digits)}`;
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
const dateFmt = new Intl.DateTimeFormat(undefined, { weekday: 'short', day: 'numeric', month: 'short' });

function tyre(compound, age) {
  const c = COMPOUNDS[compound] || COMPOUNDS.UNKNOWN;
  return `<span class="tyre" style="--c:${c.color}" title="${esc(compound)}">${c.label}</span>${
    age != null ? `<span class="age" title="Tyre age (laps) at the time">${age}L</span>` : ''
  }`;
}

const driverCell = (code, color, extra = '') =>
  `<span class="driver"><i class="swatch" style="--c:${esc(color)}"></i><b>${esc(code)}</b>${extra}</span>`;

// Delta bars: longer = further behind the fastest.
function deltaBars(rows) {
  if (!rows.length) return '<p class="muted">No data.</p>';
  const max = Math.max(...rows.map((r) => r.gap), 0.001);
  return `<div class="bars">${rows
    .map(
      (r, i) => `
      <span class="muted">${i + 1}</span>
      <span>${r.label}</span>
      <span class="track"><span class="fill" style="--c:${esc(r.color)};width:${(r.gap / max) * 100}%"></span></span>
      <span class="val">${r.gap === 0 ? `<b>${esc(r.value)}</b>` : `+${r.gap.toFixed(3)}`}</span>`
    )
    .join('')}</div>`;
}

// ---------- views ----------

// The session's fastest lap: time, who, tyre and its three sectors (purple
// where it was the fastest sector of the session). Outside races it's the
// official P1 time: the raw laps include ones deleted for track limits.
function fastestLap(d) {
  const p1 = !d.session.isRace && d.classification[0];
  if (p1 && Number.isFinite(p1.time)) {
    const row = d.perDriver.find((r) => r.driver === p1.driver);
    const lap = row?.lapList.find((l) => Math.abs(l.time - p1.time) < 0.0005);
    return { driver: p1.driver, time: p1.time, lap };
  }
  const top = d.perDriver.filter((r) => r.best).sort((a, b) => a.best.time - b.best.time)[0];
  return top && { driver: top.driver, time: top.best.time, lap: top.lapList.find((l) => l.lap === top.best.lap) };
}

function fastestLapHero(d) {
  const fl = fastestLap(d);
  if (!fl) return '';
  const info = d.drivers[fl.driver] || { code: `#${fl.driver}`, color: '#888', team: '' };
  const lap = fl.lap;
  const sectors = (lap?.sectors || [null, null, null])
    .map((s, i) => {
      const overall = s != null && d.bestSectors[i] != null && Math.abs(s - d.bestSectors[i]) < 0.0005;
      return `<div class="fl-sector${overall ? ' overall' : ''}"><div class="fl-bar"><i></i></div><span>S${i + 1} ${s != null ? s.toFixed(3) : '—'}</span></div>`;
    })
    .join('');
  return `<div class="fl-hero">
      <div class="fl-main"><small>Fastest lap</small><span class="fl-time" data-ms="${Math.round(fl.time * 1000)}">${fmtLap(fl.time)}</span></div>
      <div class="fl-side">
        <div class="fl-meta">${driverCell(info.code, info.color)}<span class="muted">${esc(info.team)}</span>${
          lap ? `${tyre(lap.compound, lap.tyreAge)}<span class="muted">Lap ${lap.lap}</span>` : ''
        }</div>
        <div class="fl-sectors">${sectors}</div>
      </div>
    </div>`;
}

function viewSummary(d) {
  const w = d.weather;
  const chips = w
    ? `<div class="chips">
        <span class="chip">Air <b>${w.air[0]}–${w.air[1]} °C</b></span>
        <span class="chip">Track <b>${w.track[0]}–${w.track[1]} °C</b></span>
        <span class="chip">Humidity <b>${Math.round(w.humidity)}%</b></span>
        <span class="chip">Wind up to <b>${w.wind} m/s</b></span>
        <span class="chip">Rain <b>${w.rain ? 'Yes' : 'None'}</b></span>
      </div>`
    : '';

  const perDriver = new Map(d.perDriver.map((r) => [r.driver, r]));
  const isRace = d.session.isRace;
  const hasPhases = d.classification.some((r) => r.phase);

  const rows = d.classification
    .map((r) => {
      const info = d.drivers[r.driver] || { code: `#${r.driver}`, name: '', team: '', color: '#888' };
      const best = perDriver.get(r.driver)?.best;
      const time = isRace ? (r.position === 1 ? fmtTotal(r.time) : '') : fmtLap(r.time);
      return `<tr>
        <td class="num muted">${r.position ?? '—'}</td>
        <td>${driverCell(info.code, info.color)}</td>
        <td class="muted">${esc(info.team)}</td>
        ${hasPhases ? `<td class="muted">${r.phase ? `Q${r.phase}` : ''}</td>` : ''}
        <td class="num">${r.status ? `<span class="status">${r.status}</span>` : time}</td>
        <td class="num muted">${r.status ? '' : fmtGap(r.gap)}</td>
        <td class="num muted">${r.laps ?? ''}</td>
        ${isRace ? '' : `<td>${best ? tyre(best.compound, best.tyreAge) : ''}</td>`}
      </tr>`;
    })
    .join('');

  const deleted = d.deletedLaps.length
    ? `<div class="card"><h2>Deleted lap times</h2><p class="hint">Mostly track limits.</p><div class="chips">${d.deletedLaps
        .map((x) => {
          const info = d.drivers[x.driver] || { code: `#${x.driver}`, color: '#888' };
          return `<span class="chip">${driverCell(info.code, info.color, ` <b>${x.count}</b>`)}</span>`;
        })
        .join('')}</div></div>`
    : '';

  const notes = d.notes.length
    ? `<div class="card"><h2>Race control</h2><ul class="notes">${d.notes
        .map(
          (n) =>
            `<li><time>${timeFmt.format(new Date(n.date))}${n.lap ? ` · L${n.lap}` : ''}</time>${esc(n.message)}</li>`
        )
        .join('')}</ul></div>`
    : '';

  return `${fastestLapHero(d)}${chips}
    <div class="grid wide-left">
      <div class="card">
        <h2>Classification</h2>
        <table>
          <thead><tr>
            <th class="num">Pos</th><th>Driver</th><th>Team</th>
            ${hasPhases ? '<th>Out in</th>' : ''}
            <th class="num">${isRace ? 'Time' : 'Best lap'}</th><th class="num">Gap</th><th class="num">Laps</th>
            ${isRace ? '' : '<th>Tyre</th>'}
          </tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
      <div>${notes || '<div class="card"><h2>Race control</h2><p class="muted">No penalties, investigations or stoppages.</p></div>'}${deleted}</div>
    </div>`;
}

function viewShort(d) {
  const teams = deltaBars(
    d.shortRuns.teams.map((t) => ({
      label: `${driverCell(d.drivers[t.driver]?.code ?? t.driver, t.color, `<span class="muted">${esc(t.team)}</span>`)}`,
      color: t.color,
      gap: t.gap,
      value: fmtLap(t.value),
    }))
  );

  const [b1, b2, b3] = d.bestSectors;
  const rows = d.perDriver
    .filter((r) => r.best)
    .sort((a, b) => a.best.time - b.best.time);
  const top = rows[0]?.best.time;
  const sector = (v, best) => `<td class="num ${v != null && v === best ? 'best' : ''}">${v != null ? v.toFixed(3) : '—'}</td>`;

  const table = rows
    .map(
      (r, i) => `<tr>
        <td class="num muted">${i + 1}</td>
        <td>${driverCell(r.code, r.color)}</td>
        <td class="num">${fmtLap(r.best.time)}</td>
        <td class="num muted">${fmtGap(r.best.time - top)}</td>
        <td>${tyre(r.best.compound, r.best.tyreAge)}</td>
        ${sector(r.sectors[0], b1)}${sector(r.sectors[1], b2)}${sector(r.sectors[2], b3)}
        <td class="num">${fmtLap(r.ideal)}</td>
        <td class="num muted">${r.ideal ? fmtSigned(r.ideal - r.best.time) : ''}</td>
      </tr>`
    )
    .join('');

  return `<p class="hint">Each driver's fastest lap. Softer tyres are quicker over one lap and fuel loads aren't public,
      so check the tyre before comparing. <b>Ideal lap</b> adds up the driver's best three sectors; the last column is
      the time left on the table. Purple = fastest sector of the session.</p>
    <div class="grid single">
      <div class="card"><h2>Teams</h2><p class="hint">Gap from each team's fastest driver to the quickest team.</p>${teams}</div>
      <div class="card">
        <h2>Drivers</h2>
        <table>
          <thead><tr><th class="num">#</th><th>Driver</th><th class="num">Best</th><th class="num">Gap</th><th>Tyre</th>
            <th class="num">S1</th><th class="num">S2</th><th class="num">S3</th><th class="num">Ideal</th><th class="num">Δ</th></tr></thead>
          <tbody>${table}</tbody>
        </table>
      </div>
    </div>`;
}

// Lap time over the laps of each run. Team-mates share a colour; the second is dashed.
function runChart(visible) {
  const W = 860;
  const H = 340;
  const P = { l: 64, r: 16, t: 14, b: 34 };
  const pushTimes = visible.flatMap((r) => r.laps.filter((l) => l.push).map((l) => l.time));
  if (!pushTimes.length) return '<p class="muted">Pick at least one driver above.</p>';

  const maxLaps = Math.max(...visible.map((r) => r.laps.length));
  let lo = Math.min(...pushTimes);
  let hi = Math.max(...pushTimes);
  const padY = Math.max(0.2, (hi - lo) * 0.08);
  lo -= padY;
  hi += padY;
  const x = (i) => P.l + (maxLaps <= 1 ? 0 : (i / (maxLaps - 1)) * (W - P.l - P.r));
  const y = (t) => P.t + ((t - lo) / (hi - lo)) * (H - P.t - P.b); // faster = higher

  const step = hi - lo > 4 ? 1 : hi - lo > 1.6 ? 0.5 : 0.25;
  let grid = '';
  for (let t = Math.ceil(lo / step) * step; t <= hi; t += step) {
    grid += `<line class="grid-line" x1="${P.l}" x2="${W - P.r}" y1="${y(t)}" y2="${y(t)}"/><text x="${P.l - 8}" y="${
      y(t) + 4
    }" text-anchor="end">${fmtLap(t)}</text>`;
  }
  for (let i = 0; i < maxLaps; i++) {
    grid += `<text x="${x(i)}" y="${H - 12}" text-anchor="middle">${i + 1}</text>`;
  }
  grid += `<text x="${(W + P.l) / 2}" y="${H}" text-anchor="middle">Lap of run</text>`;

  const seenTeams = new Set();
  const lines = visible
    .map((r) => {
      const dashed = seenTeams.has(r.team);
      seenTeams.add(r.team);
      // Break the line at traffic laps (non-push) so they don't distort it.
      const segs = [];
      let cur = [];
      r.laps.forEach((l, i) => {
        if (l.push) cur.push([x(i), y(l.time), l]);
        else if (cur.length) {
          segs.push(cur);
          cur = [];
        }
      });
      if (cur.length) segs.push(cur);
      const paths = segs
        .map(
          (s) =>
            `<polyline fill="none" stroke="${esc(r.color)}" stroke-width="2.2" ${
              dashed ? 'stroke-dasharray="6 4"' : ''
            } points="${s.map((p) => `${p[0]},${p[1]}`).join(' ')}"/>`
        )
        .join('');
      const dots = segs
        .flat()
        .map(
          (p) =>
            `<circle cx="${p[0]}" cy="${p[1]}" r="3.5" fill="${esc(r.color)}"><title>${esc(r.code)} · lap ${
              p[2].lap
            } · ${fmtLap(p[2].time)}</title></circle>`
        )
        .join('');
      return paths + dots;
    })
    .join('');

  return `<svg class="chart" viewBox="0 0 ${W} ${H}">${grid}${lines}</svg>`;
}

// Each driver's representative run at the chosen minimum length: the one with
// the most push laps (ties: the faster one).
function eligibleRuns(d) {
  const best = new Map();
  for (const r of d.longRuns.all) {
    if (r.pushLaps < state.minLaps) continue;
    const cur = best.get(r.driver);
    if (!cur || r.pushLaps > cur.pushLaps || (r.pushLaps === cur.pushLaps && r.avg < cur.avg)) best.set(r.driver, r);
  }
  return [...best.values()].sort((a, b) => a.avg - b.avg);
}

function selectedSet(runs) {
  return state.selected ?? new Set(runs.slice(0, DEFAULT_COMPARE).map((r) => r.driver));
}

function teamsFromRuns(runs) {
  const byTeam = new Map();
  for (const r of runs) if (!byTeam.has(r.team) || r.avg < byTeam.get(r.team).avg) byTeam.set(r.team, r);
  const list = [...byTeam.values()].sort((a, b) => a.avg - b.avg);
  return list.map((r) => ({ run: r, gap: r.avg - list[0].avg }));
}

function viewLong(d) {
  const s = d.longRuns.settings;
  const unit = d.session.isRace ? 'stint' : 'run';
  const method = `A long ${unit} is a block of at least <b>${state.minLaps}</b> consecutive laps within ${Math.round(
    (s.pushFactor - 1) * 100
  )}% of the driver's best lap, on one set of tyres. Out-laps, in-laps and cool-down laps end a run; a single lap lost in traffic
    is skipped. Fuel loads and engine modes aren't public, so treat this as an indication, not a verdict. <b>Trend</b> is the
    change in lap time per lap: tyre wear pushes it up, fuel burn pulls it down.`;

  let racePace = '';
  if (d.session.isRace && d.racePace.teams.length) {
    racePace = `<div class="card"><h2>Race pace</h2><p class="hint">Median of each driver's clean laps (no pit, safety car or traffic laps); best driver per team.</p>${deltaBars(
      d.racePace.teams.map((t) => ({
        label: driverCell(t.entry.code, t.color, `<span class="muted">${esc(t.team)}</span>`),
        color: t.color,
        gap: t.gap,
        value: fmtLap(t.value),
      }))
    )}</div>`;
  }

  const runs = eligibleRuns(d);
  const selected = selectedSet(runs);
  const compared = runs.filter((r) => selected.has(r.driver));

  const controls = `<div class="card controls">
      <div class="control">
        <span class="control-label">Minimum ${unit} length</span>
        <div class="seg">${MIN_LAPS_CHOICES.map(
          (n) => `<button data-minlaps="${n}" class="${n === state.minLaps ? 'active' : ''}">${n}+</button>`
        ).join('')}</div>
        <span class="muted">laps</span>
      </div>
      ${
        runs.length
          ? `<div class="control">
        <span class="control-label">Compare</span>
        <div class="legend">${runs
          .map(
            (r) =>
              `<button data-run="${r.driver}" class="${selected.has(r.driver) ? '' : 'off'}"><i class="swatch" style="--c:${esc(
                r.color
              )}"></i>${esc(r.code)}</button>`
          )
          .join('')}</div>
        <div class="seg small">
          <button data-select="top">Top ${DEFAULT_COMPARE}</button><button data-select="all">All</button><button data-select="none">Clear</button>
        </div>
      </div>`
          : ''
      }
    </div>`;

  if (!runs.length) {
    const any = d.longRuns.all.length;
    return `${racePace}${controls}<p class="hint">${method}</p><div class="state">${
      any
        ? `No ${unit}s of ${state.minLaps}+ laps in this session. Try a shorter minimum.`
        : `No long ${unit}s in this session${d.session.type === 'Qualifying' ? ' (expected in qualifying)' : ''}.`
    }</div>`;
  }

  const teams = deltaBars(
    teamsFromRuns(runs).map(({ run: r, gap }) => ({
      label: `${driverCell(r.code, r.color)} ${tyre(r.compound)} <span class="muted">${r.pushLaps} laps</span>`,
      color: r.color,
      gap,
      value: fmtLap(r.avg),
    }))
  );

  const table = compared
    .map(
      (r) => `<tr>
        <td>${driverCell(r.code, r.color)}</td>
        <td class="muted">${esc(r.team)}</td>
        <td>${tyre(r.compound, r.tyreAgeStart)}</td>
        <td class="num">${r.pushLaps}</td>
        <td class="num"><b>${fmtLap(r.avg)}</b></td>
        <td class="num muted">${fmtGap(r.avg - compared[0].avg)}</td>
        <td class="num muted">${fmtLap(r.best)}</td>
        <td class="num">${fmtSigned(r.trend)} s/lap</td>
      </tr>`
    )
    .join('');

  return `${racePace}${controls}<p class="hint">${method}</p>
    <div class="grid">
      <div class="card"><h2>${d.session.isRace ? 'Best stint per team' : 'Teams'}</h2><p class="hint">Average lap of each team's best ${unit}
        (all teams, ${state.minLaps}+ laps). Compare tyres too.</p>${teams}</div>
      <div class="card">
        <h2>Lap times through each ${unit}</h2>
        <p class="hint">The drivers picked under <b>Compare</b>. Team-mates share a colour; the second is dashed.</p>
        ${runChart(compared)}
      </div>
    </div>
    <div class="card">
      <h2>Compared ${unit}s</h2>
      ${
        compared.length
          ? `<table>
        <thead><tr><th>Driver</th><th>Team</th><th>Tyre (age at start)</th><th class="num">Laps</th><th class="num">Average</th><th class="num">Gap</th><th class="num">Best</th><th class="num">Trend</th></tr></thead>
        <tbody>${table}</tbody>
      </table>`
          : '<p class="muted">Pick drivers under Compare.</p>'
      }
    </div>`;
}

function viewTyres(d) {
  const teams = [...d.tyres].sort(
    (a, b) => Object.values(b.compounds).reduce((x, y) => x + y, 0) - Object.values(a.compounds).reduce((x, y) => x + y, 0)
  );
  const maxLaps = Math.max(...teams.map((t) => Object.values(t.compounds).reduce((x, y) => x + y, 0)), 1);
  const used = COMPOUND_ORDER.filter((c) => teams.some((t) => t.compounds[c]));

  const tyreRows = teams
    .map((t) => {
      const total = Object.values(t.compounds).reduce((x, y) => x + y, 0);
      const segs = COMPOUND_ORDER.filter((c) => t.compounds[c])
        .map(
          (c) =>
            `<span style="--c:${COMPOUNDS[c].color};width:${(t.compounds[c] / maxLaps) * 100}%" title="${c}: ${t.compounds[c]} laps"></span>`
        )
        .join('');
      return `<span>${driverCell(t.team, t.color)}</span><span class="stack">${segs}</span><span class="val">${total} laps</span>`;
    })
    .join('');

  const speeds = d.perDriver.filter((r) => r.speedTrap).sort((a, b) => b.speedTrap - a.speedTrap);
  const top = speeds[0]?.speedTrap ?? 0;
  const floor = (speeds.at(-1)?.speedTrap ?? 0) - 5;
  const speedRows = speeds
    .map(
      (r) => `<span>${driverCell(r.code, r.color)}</span>
        <span class="track"><span class="fill" style="--c:${esc(r.color)};width:${((r.speedTrap - floor) / (top - floor || 1)) * 100}%"></span></span>
        <span class="val"><b>${r.speedTrap}</b> km/h</span>
        <span class="val muted" title="Fastest intermediate 1 / intermediate 2 speeds">${r.i1 ?? '—'} / ${r.i2 ?? '—'}</span>`
    )
    .join('');

  return `<div class="grid">
      <div class="card">
        <h2>Tyre usage</h2>
        <p class="hint">Laps on each compound, both cars combined.</p>
        <div class="bars" style="grid-template-columns:auto 1fr auto">${tyreRows}</div>
        <div class="legend">${used.map((c) => `<span class="chip">${tyre(c)} ${c.toLowerCase()}</span>`).join('')}</div>
      </div>
      <div class="card">
        <h2>Top speeds</h2>
        <p class="hint">Fastest speed-trap reading per driver, then intermediate 1 / 2. High top speed often means a
          low-downforce setup (or a tow from another car).</p>
        <div class="bars" style="grid-template-columns:auto 1fr auto auto">${speedRows}</div>
      </div>
    </div>`;
}

// ---------- compare (telemetry) ----------

const teleKey = (sel) => `${state.key}/${sel.driver}/${sel.lap}`;

function defaultCompare(d) {
  const ranked = d.perDriver.filter((r) => r.best).sort((a, b) => a.best.time - b.best.time);
  const pick = (r) => r && { driver: r.driver, lap: r.best.lap };
  return { a: pick(ranked[0]), b: pick(ranked[1] || ranked[0]) };
}

function loadTelemetry(sel) {
  const key = teleKey(sel);
  if (state.tele.has(key)) return;
  state.tele.set(key, 'loading');
  const sessionKey = state.key;
  api
    .telemetry(sessionKey, sel.driver, sel.lap)
    .then((t) => state.tele.set(key, t))
    .catch((err) => state.tele.set(key, { error: err.message }))
    .finally(() => {
      if (state.key === sessionKey && state.tab === 'compare') render();
    });
}

// Linear interpolation of `key` at distance d (samples sorted by d).
function atDistance(samples, d, key) {
  let lo = 0;
  let hi = samples.length - 1;
  if (d <= samples[0].d) return samples[0][key];
  if (d >= samples[hi].d) return samples[hi][key];
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (samples[mid].d <= d) lo = mid;
    else hi = mid;
  }
  const a = samples[lo];
  const b = samples[hi];
  return a[key] + ((b[key] - a[key]) * (d - a.d)) / (b.d - a.d || 1);
}

// Prepared traces for the chart: B's distance is scaled onto A's lap length so
// both traces line up, and the gap is computed on a fixed distance grid.
function buildTraces(ta, tb) {
  const L = ta.length;
  const scale = L / tb.length;
  const bs = tb.samples.map((x) => ({ ...x, d: x.d * scale }));
  const N = 500;
  const delta = [];
  for (let i = 0; i <= N; i++) {
    const d = (L * i) / N;
    delta.push({ d, v: atDistance(bs, d, 't') - atDistance(ta.samples, d, 't') });
  }
  return { L, a: ta.samples, b: bs, delta };
}

const CHART = { W: 860, l: 54, r: 14 };
const PANELS = [
  { key: 'speed', label: 'Speed (km/h)', h: 190 },
  { key: 'delta', label: 'Gap B to A (s)', h: 90 },
  { key: 'throttle', label: 'Throttle %', h: 64 },
  { key: 'brake', label: 'Brake', h: 30 },
  { key: 'gear', label: 'Gear', h: 64 },
];
const PANEL_GAP = 16;

function telemetryChart(tr, colA, colB, dashedB, sectorDs) {
  const { W, l, r } = CHART;
  const x = (d) => l + (d / tr.L) * (W - l - r);
  let y0 = 8;
  let out = '';
  const line = (pts, color, dashed, width = 1.8) =>
    `<polyline fill="none" stroke="${esc(color)}" stroke-width="${width}" stroke-linejoin="round" ${
      dashed ? 'stroke-dasharray="5 4"' : ''
    } points="${pts.map((p) => `${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(' ')}"/>`;

  for (const p of PANELS) {
    const top = y0;
    const bottom = y0 + p.h;
    out += `<text class="panel-label" x="${l}" y="${top - 1}">${p.label}</text>`;
    if (p.key === 'speed' || p.key === 'throttle' || p.key === 'gear') {
      const vals = [...tr.a, ...tr.b].map((s) => s[p.key]);
      const lo = p.key === 'speed' ? Math.floor(Math.min(...vals) / 50) * 50 : 0;
      const hi = p.key === 'speed' ? Math.ceil(Math.max(...vals) / 50) * 50 : p.key === 'gear' ? 8 : 100;
      const y = (v) => bottom - ((v - lo) / (hi - lo || 1)) * (p.h - 6);
      const ticks = p.key === 'speed' ? [lo, (lo + hi) / 2, hi] : p.key === 'gear' ? [2, 4, 6, 8] : [0, 100];
      for (const t of ticks) {
        out += `<line class="grid-line" x1="${l}" x2="${W - r}" y1="${y(t)}" y2="${y(t)}"/><text x="${l - 6}" y="${y(t) + 4}" text-anchor="end">${t}</text>`;
      }
      // Gear is a step trace.
      const pts = (arr) =>
        p.key === 'gear'
          ? arr.flatMap((s, i) => (i ? [[x(s.d), y(arr[i - 1].gear)], [x(s.d), y(s.gear)]] : [[x(s.d), y(s.gear)]]))
          : arr.map((s) => [x(s.d), y(s[p.key])]);
      out += line(pts(tr.a), colA, false) + line(pts(tr.b), colB, dashedB);
    } else if (p.key === 'delta') {
      const vals = tr.delta.map((s) => s.v);
      const m = Math.max(0.1, ...vals.map(Math.abs)) * 1.1;
      const y = (v) => top + ((m - v) / (2 * m)) * p.h;
      out += `<line class="grid-line zero" x1="${l}" x2="${W - r}" y1="${y(0)}" y2="${y(0)}"/>`;
      out += `<text x="${l - 6}" y="${y(m * 0.8) + 4}" text-anchor="end">+${(m * 0.8).toFixed(2)}</text>`;
      out += `<text x="${l - 6}" y="${y(-m * 0.8) + 4}" text-anchor="end">−${(m * 0.8).toFixed(2)}</text>`;
      out += line(tr.delta.map((s) => [x(s.d), y(s.v)]), colB, false, 2);
    } else if (p.key === 'brake') {
      const bars = (arr, color, row) => {
        let html = '';
        const h = p.h / 2 - 3;
        const yy = top + row * (p.h / 2);
        for (let i = 0; i < arr.length - 1; i++) {
          if (arr[i].brake) html += `<rect x="${x(arr[i].d)}" y="${yy}" width="${Math.max(1, x(arr[i + 1].d) - x(arr[i].d))}" height="${h}" fill="${esc(color)}" opacity="0.9"/>`;
        }
        return html;
      };
      out += bars(tr.a, colA, 0) + bars(tr.b, colB, 1);
      out += `<text x="${l - 6}" y="${top + 10}" text-anchor="end">A</text><text x="${l - 6}" y="${top + p.h / 2 + 10}" text-anchor="end">B</text>`;
    }
    y0 = bottom + PANEL_GAP;
  }
  const H = y0 + 10;

  // Sector boundaries (from A's sector times) and the distance axis.
  for (const [i, d] of sectorDs.entries()) {
    out += `<line class="sector-line" x1="${x(d)}" x2="${x(d)}" y1="0" y2="${H - 22}"/><text class="sector-label" x="${x(d) + 4}" y="${H - 24}">S${i + 2}</text>`;
  }
  for (let km = 0; km <= tr.L / 1000; km += tr.L > 6000 ? 1 : 0.5) {
    out += `<text x="${x(km * 1000)}" y="${H - 6}" text-anchor="middle">${km} km</text>`;
  }
  out += `<line id="cmpCursor" class="cursor-line" x1="0" x2="0" y1="0" y2="${H - 20}" visibility="hidden"/>`;
  return `<svg id="cmpChart" class="chart telemetry" viewBox="0 0 ${W} ${H}" data-l="${l}" data-r="${r}" data-w="${W}">${out}</svg>`;
}

const MINI_SECTORS = 25;

function loadLocation(sel) {
  const key = teleKey(sel);
  if (state.locs.has(key)) return;
  state.locs.set(key, 'loading');
  const sessionKey = state.key;
  api
    .location(sessionKey, sel.driver, sel.lap)
    .then((l) => state.locs.set(key, l))
    .catch((err) => state.locs.set(key, { error: err.message }))
    .finally(() => {
      if (state.key === sessionKey && state.tab === 'compare') render();
    });
}

// A's lap drawn as the track, each mini-sector coloured by whoever was faster
// through it (from the gap trace).
function miniSectorMap(loc, ta, tr, colA, colB) {
  const byT = [...ta.samples].sort((x, y) => x.t - y.t);
  const dAt = (t) => {
    const i = byT.findIndex((x) => x.t >= t);
    if (i <= 0) return i === 0 ? byT[0].d : byT.at(-1).d;
    const a = byT[i - 1];
    const b = byT[i];
    return a.d + ((b.d - a.d) * (t - a.t)) / (b.t - a.t || 1);
  };
  const pts = loc.points.map((p) => ({ x: p.x, y: p.y, d: dAt(p.t) }));
  const gapAt = (d) => atDistance(tr.delta, d, 'v');
  const winner = [];
  for (let k = 0; k < MINI_SECTORS; k++) {
    const d0 = (tr.L * k) / MINI_SECTORS;
    const d1 = (tr.L * (k + 1)) / MINI_SECTORS;
    winner.push(gapAt(d1) - gapAt(d0) > 0 ? 'a' : 'b'); // gap grows = B lost time
  }
  const W = 360;
  const H = 250;
  const project = window.TrackMap.fit(pts, W, H, 14);
  const colour = { a: colA, b: colA === colB ? '#ffffff' : colB };
  let paths = '';
  let cur = null;
  let seg = [];
  const flush = () => {
    if (seg.length > 1) {
      paths += `<polyline fill="none" stroke="${esc(colour[cur])}" stroke-width="6" stroke-linecap="round" stroke-linejoin="round" points="${seg
        .map((p) => project(p.x, p.y).map((v) => v.toFixed(1)).join(','))
        .join(' ')}"/>`;
    }
  };
  for (const p of pts) {
    const k = Math.min(MINI_SECTORS - 1, Math.max(0, Math.floor((p.d / tr.L) * MINI_SECTORS)));
    const w = winner[k];
    if (w !== cur) {
      if (cur) {
        seg.push(p); // join segments without gaps
        flush();
      }
      cur = w;
      seg = [p];
    } else seg.push(p);
  }
  flush();
  const [sx, sy] = project(pts[0].x, pts[0].y);
  const aCount = winner.filter((w) => w === 'a').length;
  return {
    svg: `<svg class="track-map" viewBox="0 0 ${W} ${H}"><path d="${window.TrackMap.pathOf(pts, project)}" fill="none" stroke="rgba(255,255,255,0.12)" stroke-width="12" stroke-linejoin="round"/>${paths}<circle cx="${sx}" cy="${sy}" r="5" fill="#e10600" stroke="#000" stroke-width="1.5"/></svg>`,
    aCount,
    bCount: MINI_SECTORS - aCount,
    colour,
  };
}

function lapLabel(l, bestLap) {
  return `L${l.lap} · ${fmtLap(l.time)}${l.pitOut ? ' · out-lap' : ''}${l.deleted ? ' · deleted' : ''}${l.lap === bestLap ? ' · fastest' : ''}`;
}

function viewCompare(d) {
  if (!state.cmp) state.cmp = defaultCompare(d);
  const { a, b } = state.cmp;
  if (!a || !b) return '<div class="state">No timed laps in this session.</div>';
  const rowOf = (sel) => d.perDriver.find((r) => r.driver === sel.driver);
  const ra = rowOf(a);
  const rb = rowOf(b);
  const la = ra.lapList.find((l) => l.lap === a.lap);
  const lb = rb.lapList.find((l) => l.lap === b.lap);
  const dashedB = ra.color === rb.color;

  const picker = (side, sel, row, lap) => `
    <div class="cmp-side">
      <span class="cmp-tag" style="--c:${esc(row.color)}">${side.toUpperCase()}</span>
      <button class="pick-btn" data-pick="${side}-driver">${driverCell(row.code, row.color)} <span class="caret">▾</span></button>
      <button class="pick-btn" data-pick="${side}-lap">${esc(lap ? lapLabel(lap, row.best?.lap) : `L${sel.lap}`)} <span class="caret">▾</span></button>
      ${lap ? tyre(lap.compound, lap.tyreAge) : ''}
    </div>`;

  // Lap / sector table: the faster value of each row is highlighted.
  const cmpRow = (label, va, vb, fmt, lowerIsBetter = true) => {
    if (va == null || vb == null) return `<tr><td>${label}</td><td class="num">${va == null ? '—' : fmt(va)}</td><td class="num">${vb == null ? '—' : fmt(vb)}</td><td></td></tr>`;
    const aWins = lowerIsBetter ? va < vb : va > vb;
    const bWins = lowerIsBetter ? vb < va : vb > va;
    const diff = vb - va;
    return `<tr><td>${label}</td>
      <td class="num ${aWins ? 'win' : ''}" style="--c:${esc(ra.color)}">${fmt(va)}</td>
      <td class="num ${bWins ? 'win' : ''}" style="--c:${esc(rb.color)}">${fmt(vb)}</td>
      <td class="num muted">${diff === 0 ? '—' : lowerIsBetter ? fmtSigned(diff) : `${diff > 0 ? '+' : '−'}${Math.abs(diff)}`}</td></tr>`;
  };
  const sec = (v) => v.toFixed(3);
  const table = `<table class="cmp-table">
      <thead><tr><th></th><th class="num">${driverCell(ra.code, ra.color)}</th><th class="num">${driverCell(rb.code, rb.color)}</th><th class="num">B − A</th></tr></thead>
      <tbody>
        ${cmpRow('Lap', la?.time, lb?.time, fmtLap)}
        ${cmpRow('Sector 1', la?.sectors[0], lb?.sectors[0], sec)}
        ${cmpRow('Sector 2', la?.sectors[1], lb?.sectors[1], sec)}
        ${cmpRow('Sector 3', la?.sectors[2], lb?.sectors[2], sec)}
        ${cmpRow('Speed trap (km/h)', la?.speedTrap, lb?.speedTrap, String, false)}
      </tbody></table>`;

  // Telemetry for both laps.
  loadTelemetry(a);
  loadTelemetry(b);
  const ta = state.tele.get(teleKey(a));
  const tb = state.tele.get(teleKey(b));
  let chart;
  let mapCard = '<div class="card"><h2>Faster by mini-sector</h2><div class="state">Loading track…</div></div>';
  if (ta === 'loading' || tb === 'loading') chart = '<div class="state">Loading car telemetry…</div>';
  else if (ta?.error || tb?.error) chart = `<div class="state error">${esc(ta?.error || tb?.error)}</div>`;
  else {
    state.traces = buildTraces(ta, tb);
    // Sector boundaries: where A had driven for S1 and S1+S2 seconds.
    const sectorDs = [];
    if (la?.sectors[0] && la?.sectors[1]) {
      const byT = (t) => {
        const i = ta.samples.findIndex((x) => x.t >= t);
        return i > 0 ? ta.samples[i].d : null;
      };
      for (const t of [la.sectors[0], la.sectors[0] + la.sectors[1]]) {
        const dd = byT(t);
        if (dd != null) sectorDs.push(dd);
      }
    }
    chart = `<div class="readout" id="cmpReadout">Hover the chart to read values along the lap.</div>
      ${telemetryChart(state.traces, ra.color, rb.color, dashedB, sectorDs)}`;

    loadLocation(a);
    const loc = state.locs.get(teleKey(a));
    if (loc && loc !== 'loading' && !loc.error) {
      const m = miniSectorMap(loc, ta, state.traces, ra.color, rb.color);
      mapCard = `<div class="card"><h2>Faster by mini-sector</h2>
        <p class="hint">The lap split into ${MINI_SECTORS} equal parts, each coloured by the quicker driver.</p>
        ${m.svg}
        <div class="legend">
          <span class="chip"><i class="swatch" style="--c:${esc(m.colour.a)}"></i> ${esc(ra.code)} <b>${m.aCount}</b></span>
          <span class="chip"><i class="swatch" style="--c:${esc(m.colour.b)}"></i> ${esc(rb.code)} <b>${m.bCount}</b></span>
        </div></div>`;
    } else if (loc?.error) {
      mapCard = `<div class="card"><h2>Faster by mini-sector</h2><div class="state">${esc(loc.error)}</div></div>`;
    }
  }

  return `<div class="card controls cmp-pickers">${picker('a', a, ra, la)}<span class="vs">vs</span>${picker('b', b, rb, lb)}</div>
    <div class="grid cmp-grid">
      <div class="card"><h2>Lap comparison</h2><p class="hint">Highlighted = faster. B − A: negative means B was quicker.</p>${table}</div>
      ${mapCard}
    </div>
    <div class="card"><p class="hint">Traces are plotted against distance around the lap, so the two drivers line up corner by corner.
        <b>Gap B to A</b> rises where B loses time and falls where B gains it. Telemetry is sampled about 4 times a second, so short
        braking spikes can be smoothed out. ${dashedB ? 'Team-mates share a colour; B is dashed.' : ''}</p>${chart}</div>`;
}

// Crosshair: follow the mouse along the chart and read values for both laps.
function bindCompareChart() {
  const svg = document.getElementById('cmpChart');
  const out = document.getElementById('cmpReadout');
  if (!svg || !out || !state.traces) return;
  const tr = state.traces;
  const d = state.data;
  const code = (sel) => d.drivers[sel.driver]?.code || sel.driver;
  const W = Number(svg.dataset.w);
  const l = Number(svg.dataset.l);
  const r = Number(svg.dataset.r);
  const cursor = document.getElementById('cmpCursor');
  svg.addEventListener('mousemove', (e) => {
    const box = svg.getBoundingClientRect();
    const vx = ((e.clientX - box.left) / box.width) * W;
    if (vx < l || vx > W - r) return;
    const dist = ((vx - l) / (W - l - r)) * tr.L;
    cursor.setAttribute('x1', vx);
    cursor.setAttribute('x2', vx);
    cursor.setAttribute('visibility', 'visible');
    const v = (arr, k) => atDistance(arr, dist, k);
    const gap = atDistance(tr.delta.map((x) => ({ d: x.d, v: x.v })), dist, 'v');
    const { a, b } = state.cmp;
    out.innerHTML = `<b>${(dist / 1000).toFixed(2)} km</b>
      <span>${esc(code(a))} <b>${Math.round(v(tr.a, 'speed'))}</b> km/h · ${Math.round(v(tr.a, 'throttle'))}% · G${Math.round(v(tr.a, 'gear'))}</span>
      <span>${esc(code(b))} <b>${Math.round(v(tr.b, 'speed'))}</b> km/h · ${Math.round(v(tr.b, 'throttle'))}% · G${Math.round(v(tr.b, 'gear'))}</span>
      <span>Gap <b>${fmtSigned(gap)}</b> s</span>`;
  });
  svg.addEventListener('mouseleave', () => cursor.setAttribute('visibility', 'hidden'));
}

async function pickForCompare(what) {
  const d = state.data;
  const [side, kind] = what.split('-');
  const sel = state.cmp[side];
  let value;
  if (kind === 'driver') {
    const ranked = d.perDriver.filter((r) => r.best).sort((x, y) => x.best.time - y.best.time);
    value = await api.choose(
      ranked.map((r) => ({ label: `${r.code}   ${r.team}   ${fmtLap(r.best.time)}`, value: r.driver, checked: r.driver === sel.driver }))
    );
    if (value == null) return;
    const row = d.perDriver.find((r) => r.driver === value);
    state.cmp[side] = { driver: value, lap: row.best.lap };
  } else {
    const row = d.perDriver.find((r) => r.driver === sel.driver);
    value = await api.choose(
      row.lapList.map((l) => ({ label: `${lapLabel(l, row.best?.lap)}   ${l.compound.toLowerCase()}`, value: l.lap, checked: l.lap === sel.lap }))
    );
    if (value == null) return;
    state.cmp[side] = { driver: sel.driver, lap: value };
  }
  render();
}

// ---------- replay ----------

const REPLAY_SPEEDS = [1, 4, 16, 32];
const MAP_W = 620;
const MAP_H = 430;

function replayState() {
  if (!state.replay || state.replay.key !== state.key) {
    state.replay = { key: state.key, info: null, error: null, t: 0, playing: false, speed: 4, chunks: new Map(), raf: 0, last: 0, slow: 0 };
  }
  return state.replay;
}

const fmtClock = (ms) => {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
};

// Latest entry with time <= t in a list sorted by time (index 0 = time).
function lastAtOrBefore(list, t, timeOf = (x) => x[0]) {
  let lo = 0;
  let hi = list.length - 1;
  let ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (timeOf(list[mid]) <= t) {
      ans = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return ans;
}

function prepareReplay(info) {
  // Per-driver position and lap histories for quick lookups.
  const byDriver = (rows) => {
    const m = new Map();
    for (const [t, d, v] of rows) {
      if (!m.has(d)) m.set(d, []);
      m.get(d).push([t, v]);
    }
    return m;
  };
  info.posBy = byDriver(info.positions);
  info.lapBy = byDriver(info.laps);
  // Generous padding leaves room for car labels at the edges.
  info.project = info.track ? window.TrackMap.fit(info.track.points, MAP_W, MAP_H, 40) : null;
  return info;
}

function loadChunk(r, index) {
  if (index < 0 || index * r.info.chunkMs > r.info.duration || r.chunks.has(index)) return;
  r.chunks.set(index, 'loading');
  api
    .replayChunk(r.key, index)
    .then((c) => r.chunks.set(index, c))
    .catch(() => r.chunks.delete(index)); // retried on the next frame
}

// Interpolated [x, y] of a car at time t, or null if it isn't on track.
function carAt(r, driver, t) {
  const idx = Math.floor(t / r.info.chunkMs);
  const chunk = r.chunks.get(idx);
  if (!chunk || chunk === 'loading') return null;
  const list = chunk.cars[driver];
  if (!list?.length) return null;
  const i = lastAtOrBefore(list, t);
  if (i < 0) return null;
  const a = list[i];
  let b = list[i + 1];
  if (!b) {
    const next = r.chunks.get(idx + 1);
    b = next && next !== 'loading' ? next.cars[driver]?.[0] : null;
  }
  if (t - a[0] > 4000) return null; // no recent sample: in the garage
  if (!b || b[0] - a[0] > 4000) return [a[1], a[2]];
  const f = (t - a[0]) / (b[0] - a[0] || 1);
  return [a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}

function runningOrder(info, t) {
  const rows = [];
  for (const [d, list] of info.posBy) {
    const i = lastAtOrBefore(list, t);
    if (i >= 0) rows.push({ driver: d, pos: list[i][1] });
  }
  return rows.sort((a, b) => a.pos - b.pos);
}

function viewReplay() {
  const r = replayState();
  if (!r.info && !r.error) {
    api
      .replayInfo(r.key)
      .then((info) => {
        r.info = prepareReplay(info);
      })
      .catch((err) => {
        r.error = err.message;
      })
      .finally(() => {
        if (state.tab === 'replay' && state.key === r.key) render();
      });
    return '<div class="state">Loading the session…</div>';
  }
  if (r.error) return `<div class="state error">${esc(r.error)}</div>`;
  if (!r.info) return '<div class="state">Loading the session…</div>';
  const info = r.info;
  if (!info.project) return '<div class="state">No position data to draw this session.</div>';

  const cars = Object.entries(info.drivers)
    .map(
      ([num, d]) => `<g class="car" data-car="${num}" visibility="hidden">
        <circle r="7" fill="${esc(d.color)}" stroke="#000" stroke-width="1.5"/>
        <text x="10" y="4">${esc(d.code)}</text></g>`
    )
    .join('');
  const [sx, sy] = info.project(info.track.points[0].x, info.track.points[0].y);

  return `<div class="replay-grid">
      <div class="card replay-map">
        <svg viewBox="0 0 ${MAP_W} ${MAP_H}" class="track-map">
          <path d="${window.TrackMap.pathOf(info.track.points, info.project)}" fill="none" stroke="rgba(255,255,255,0.16)" stroke-width="14" stroke-linejoin="round"/>
          <path d="${window.TrackMap.pathOf(info.track.points, info.project)}" fill="none" stroke="rgba(255,255,255,0.45)" stroke-width="2" stroke-linejoin="round"/>
          <rect x="${sx - 2}" y="${sy - 9}" width="4" height="18" fill="#e10600"/>
          <g id="rpCars">${cars}</g>
        </svg>
        <div class="rp-msg" id="rpMsg"></div>
        <div class="rp-loading" id="rpLoading">Loading car positions…</div>
      </div>
      <div class="card replay-tower"><h2>Running order</h2><ol id="rpTower"></ol></div>
    </div>
    <div class="card replay-controls">
      <button class="rp-play" id="rpPlay" title="Play / pause">${r.playing ? '❚❚' : '▶'}</button>
      <div class="seg">${REPLAY_SPEEDS.map((v) => `<button data-speed="${v}" class="${v === r.speed ? 'active' : ''}">${v}×</button>`).join('')}</div>
      <input type="range" id="rpSeek" min="0" max="${info.duration}" step="1000" value="${r.t}"/>
      <span class="rp-clock" id="rpClock"></span>
    </div>`;
}

// Draw cars, running order, clock and messages for time t.
function drawReplay(r, full) {
  const info = r.info;
  const t = r.t;
  let missing = false;
  for (const g of document.querySelectorAll('#rpCars .car')) {
    const pos = carAt(r, Number(g.dataset.car), t);
    if (!pos) {
      g.setAttribute('visibility', 'hidden');
      if (!r.chunks.get(Math.floor(t / info.chunkMs)) || r.chunks.get(Math.floor(t / info.chunkMs)) === 'loading') missing = true;
      continue;
    }
    const [x, y] = info.project(pos[0], pos[1]);
    g.setAttribute('transform', `translate(${x.toFixed(1)},${y.toFixed(1)})`);
    g.setAttribute('visibility', 'visible');
  }
  const loading = document.getElementById('rpLoading');
  if (loading) loading.style.visibility = missing ? 'visible' : 'hidden';
  if (!full) return;

  const order = runningOrder(info, t);
  const tower = document.getElementById('rpTower');
  if (tower) {
    tower.innerHTML = order
      .map((o) => {
        const d = info.drivers[o.driver] || { code: `#${o.driver}`, color: '#888' };
        return `<li><span class="pos">${o.pos}</span><i class="swatch" style="--c:${esc(d.color)}"></i><b>${esc(d.code)}</b></li>`;
      })
      .join('');
  }
  const clock = document.getElementById('rpClock');
  if (clock) {
    let lapText = '';
    if (info.isRace && order[0]) {
      const laps = info.lapBy.get(order[0].driver) || [];
      const i = lastAtOrBefore(laps, t);
      if (i >= 0) lapText = ` · Lap ${laps[i][1]}`;
    }
    clock.textContent = `${fmtClock(t)} / ${fmtClock(info.duration)}${lapText}`;
  }
  const seek = document.getElementById('rpSeek');
  if (seek && !r.seeking) seek.value = t;
  const msgEl = document.getElementById('rpMsg');
  if (msgEl) {
    const i = lastAtOrBefore(info.messages, t, (m) => m.t);
    const m = i >= 0 && t - info.messages[i].t < 12000 ? info.messages[i] : null;
    msgEl.textContent = m ? m.message : '';
    msgEl.className = `rp-msg ${m ? 'show' : ''} ${m?.flag ? `flag-${String(m.flag).toLowerCase().replace(/\s+/g, '-')}` : ''}`;
  }
}

function replayVisible() {
  return state.tab === 'replay' && !!document.getElementById('rpCars') && document.getElementById('island').classList.contains('big');
}

function replayLoop(now) {
  const r = state.replay;
  if (!r?.info || !replayVisible()) {
    if (r) {
      r.raf = 0;
      r.playing = false;
    }
    return;
  }
  const dt = r.last ? now - r.last : 0;
  r.last = now;
  const idx = Math.floor(r.t / r.info.chunkMs);
  loadChunk(r, idx);
  loadChunk(r, idx + 1);
  const ready = r.chunks.get(idx) && r.chunks.get(idx) !== 'loading';
  // Hold the clock while the current minute is still loading.
  if (r.playing && ready) {
    r.t = Math.min(r.info.duration, r.t + dt * r.speed);
    if (r.t >= r.info.duration) {
      r.playing = false;
      const btn = document.getElementById('rpPlay');
      if (btn) btn.textContent = '▶';
    }
  }
  r.slow += dt;
  const full = r.slow > 400;
  if (full) r.slow = 0;
  drawReplay(r, full);
  r.raf = requestAnimationFrame(replayLoop);
}

function startReplay() {
  const r = state.replay;
  if (!r?.info) return;
  if (!r.raf) {
    r.last = 0;
    r.raf = requestAnimationFrame(replayLoop);
  }
  drawReplay(r, true);
  const seek = document.getElementById('rpSeek');
  if (seek) {
    seek.addEventListener('input', () => {
      r.seeking = true;
      r.t = Number(seek.value);
      drawReplay(r, true);
    });
    seek.addEventListener('change', () => {
      r.seeking = false;
    });
  }
}

// ---------- rendering & loading ----------

const VIEWS = { summary: viewSummary, short: viewShort, long: viewLong, compare: viewCompare, replay: viewReplay, tyres: viewTyres };

function render() {
  for (const b of $('bigTabs').querySelectorAll('button')) b.classList.toggle('active', b.dataset.tab === state.tab);
  const longTab = $('bigTabs').querySelector('[data-tab="long"]');
  longTab.textContent = state.data?.session.isRace ? 'Race pace' : 'Long runs';
  if (state.data) {
    const scroll = $('bigBody').scrollTop;
    $('bigBody').innerHTML = VIEWS[state.tab](state.data);
    $('bigBody').scrollTop = scroll;
    if (state.tab === 'compare') bindCompareChart();
    if (state.tab === 'replay') startReplay();
    // Counts up each time the Summary is opened or switched to, not on other redraws.
    if (state.tab === 'summary' && state.playLap) {
      state.playLap = false;
      playFastestLap();
    }
  }
}

// Opening a session: the fastest-lap timer counts up to the real time, the
// sectors fill, then it turns purple; the rest of the page fades in under it.
function playFastestLap() {
  const body = $('bigBody');
  const el = body.querySelector('.fl-time');
  if (!el || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const target = Number(el.dataset.ms);
  const fills = body.querySelectorAll('.fl-bar i');
  const fmt = (ms) => `${Math.floor(ms / 60000)}:${((ms % 60000) / 1000).toFixed(3).padStart(6, '0')}`;
  body.classList.remove('fl-done');
  body.classList.add('fl-enter');
  const t0 = performance.now();
  const DURATION = 800;
  const step = (now) => {
    if (!el.isConnected) return;
    const k = Math.min(1, (now - t0) / DURATION);
    const e = 1 - (1 - k) ** 3; // ease out
    el.textContent = fmt(Math.round(target * e));
    fills.forEach((f, i) => (f.style.width = `${Math.max(0, Math.min(1, e * 3 - i)) * 100}%`));
    if (k < 1) requestAnimationFrame(step);
    else {
      body.classList.add('fl-done');
      setTimeout(() => body.classList.remove('fl-enter'), 1200);
    }
  };
  requestAnimationFrame(step);
}

// The "paused during a live session" note is expected, not a fault: show it
// plainly; anything else is an error.
function showError(err, prefix = '') {
  const paused = /pauses while a session is live/i.test(err.message);
  setStatus(paused ? esc(err.message) : `${prefix}${esc(err.message)}`, !paused);
}

function setStatus(html, isError = false) {
  $('bigBody').innerHTML = `<div class="state ${isError ? 'error' : ''}">${html}</div>`;
}

// While a session loads: a lap timer runs and the three sectors fill
// yellow, green, then purple. The timer stops once it leaves the page.
const LAP_MS = 3000; // one animated lap
function showLapLoading(caption) {
  setStatus(`<div class="lap-loading">
      <div class="lap-time"><small>Setting a lap</small><span>0:00.000</span></div>
      <div class="lap-sectors"><div><i></i></div><div><i></i></div><div><i></i></div></div>
      <div class="lap-cap">${esc(caption)}</div>
    </div>`);
  const el = $('bigBody').querySelector('.lap-time span');
  if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    el.textContent = '1:35.291';
    return;
  }
  const t0 = performance.now();
  const tick = (now) => {
    if (!el.isConnected) return;
    const ms = (((now - t0) % LAP_MS) / LAP_MS) * 95291; // counts to a 1:35.291 lap
    el.textContent = `${Math.floor(ms / 60000)}:${((ms % 60000) / 1000).toFixed(3).padStart(6, '0')}`;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

async function load(key) {
  const token = ++state.loadToken;
  state.key = key;
  state.loading = true;
  state.data = null;
  state.selected = null; // back to the default comparison for each session
  state.cmp = null;
  state.traces = null;
  const s = state.sessions.find((x) => x.key === key);
  $('bigTitle').textContent = s ? `${s.meeting} · ${s.name}` : 'Session';
  $('bigSub').textContent = s ? `${s.location} · ${dateFmt.format(new Date(s.start))}` : '';
  showLapLoading('Loading laps, tyres and timing… (takes a few seconds)');
  try {
    const data = await api.get(key);
    if (token !== state.loadToken) return;
    state.loading = false;
    state.data = data;
    state.playLap = true;
    render();
  } catch (err) {
    if (token !== state.loadToken) return;
    state.loading = false;
    showError(err);
  }
}

function sessionForStart(start) {
  const t = Date.parse(start);
  return state.sessions.find((s) => Math.abs(Date.parse(s.start) - t) < 30 * 60e3);
}

let initialized = false;

async function ensureSessions() {
  if (state.sessions.length) return true;
  try {
    state.sessions = await api.sessions();
  } catch (err) {
    $('bigTitle').textContent = 'Session analysis';
    showError(err, "Couldn't load the session list: ");
    return false;
  }
  if (!state.sessions.length) {
    $('bigTitle').textContent = 'Session analysis';
    setStatus('No completed sessions this season yet.');
    return false;
  }
  return true;
}

// Show the view for the session starting at `start` (ISO), or keep the
// current one, or fall back to the latest completed session.
async function show(start) {
  if (!initialized) {
    initialized = true;
    wire();
  }
  if (!(await ensureSessions())) return;
  state.playLap = true;
  const wanted = (start && sessionForStart(start)?.key) || state.key || state.sessions[0].key;
  if (wanted !== state.key || (!state.data && !state.loading)) load(wanted);
  else if (state.data) render();
}

function wire() {
  $('bigTabs').addEventListener('click', (e) => {
    const b = e.target.closest('button[data-tab]');
    if (!b) return;
    state.tab = b.dataset.tab;
    state.playLap = state.tab === 'summary';
    render();
    $('bigBody').scrollTop = 0;
  });
  $('bigBody').addEventListener('click', (e) => {
    const d = state.data;
    if (!d) return;
    const r = state.replay;
    if (e.target.closest('#rpPlay') && r?.info) {
      if (r.t >= r.info.duration) r.t = 0;
      r.playing = !r.playing;
      e.target.closest('#rpPlay').textContent = r.playing ? '❚❚' : '▶';
      startReplay();
      return;
    }
    const speedBtn = e.target.closest('button[data-speed]');
    if (speedBtn && r) {
      r.speed = Number(speedBtn.dataset.speed);
      for (const b of speedBtn.parentElement.children) b.classList.toggle('active', b === speedBtn);
      return;
    }
    const pickBtn = e.target.closest('button[data-pick]');
    if (pickBtn) {
      pickForCompare(pickBtn.dataset.pick);
      return;
    }
    const run = e.target.closest('button[data-run]');
    const min = e.target.closest('button[data-minlaps]');
    const pick = e.target.closest('button[data-select]');
    if (!run && !min && !pick) return;
    // Keep the scroll position: these controls re-render the tab.
    const scroll = $('bigBody').scrollTop;
    if (run) {
      const sel = new Set(selectedSet(eligibleRuns(d)));
      const id = Number(run.dataset.run);
      if (sel.has(id)) sel.delete(id);
      else sel.add(id);
      state.selected = sel;
    } else if (min) {
      state.minLaps = Number(min.dataset.minlaps);
    } else {
      const runs = eligibleRuns(d);
      state.selected =
        pick.dataset.select === 'all' ? new Set(runs.map((r) => r.driver)) : pick.dataset.select === 'none' ? new Set() : null;
    }
    render();
    $('bigBody').scrollTop = scroll;
  });
  $('bigSession').addEventListener('click', async () => {
    const key = await api.pick(state.key);
    if (key && key !== state.key) load(key);
  });
}

window.AnalysisView = { show };
})();
