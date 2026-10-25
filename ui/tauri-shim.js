// Implements the `window.f1` interface the page expects (in the Electron app, preload.js
// provides it) on top of Tauri's `invoke` (page -> Rust) and `listen` (Rust -> page).
// Features not yet ported reject or do nothing, so the page degrades instead of breaking.
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  const on = (name, cb, pick = (e) => e.payload) => {
    listen(name, (e) => cb(pick(e)));
  };
  const unsupported = (what) => () => Promise.reject(new Error(`${what} is not available yet`));
  const noop = () => {};
  const iso = (ms) => (ms == null ? null : new Date(ms).toISOString());

  async function snapshot(opts = {}) {
    const result = {
      fetchedAt: Date.now(),
      weekend: null,
      lastRace: null,
      standings: null,
      constructors: null,
      circuit: null,
      title: null,
      error: null,
    };
    try {
      const [weekend, standings] = await Promise.all([
        invoke('weekend'),
        invoke('standings'),
      ]);
      if (weekend) {
        weekend.sessions = weekend.sessions.map((s) => ({
          ...s,
          start: iso(s.startMs),
          end: iso(s.endMs),
        }));
      }
      result.weekend = weekend;
      if (standings) {
        result.lastRace = standings.lastRace ?? null;
        result.standings = standings.drivers ? standings.drivers.rows : [];
        result.constructors = standings.constructors ?? [];
      }
      if (opts.extras === false) {
        result.extrasPending = true;
      } else if (weekend && weekend.circuitId) {
        result.circuit = await invoke('circuit_history', { circuitId: weekend.circuitId, season: weekend.season }).catch(
          () => null,
        );
      }
    } catch (err) {
      result.error = String(err);
    }
    return result;
  }

  window.f1 = {
    snapshot,
    news: () => invoke('news'),
    trackMap: unsupported('Track map'),
    weekendResults: () => invoke('weekend_results'),
    openArticle: (url) => invoke('open_article', { url }),
    settings: () =>
      Promise.resolve({
        autoHide: false,
        notchAttached: true,
        hotkey: 'CommandOrControl+Alt+F',
        position: null,
        docked: false,
      }),
    notch: () => Promise.resolve(null),
    onNotch: noop,
    demo: () => Promise.resolve(false),
    onDemo: noop,
    setInteractive: (v) => invoke('set_interactive', { on: !!v }),
    dragStart: noop,
    dragEnd: noop,
    openMenu: noop,
    setBig: () => Promise.resolve(),
    setRect: (rect) => invoke('report_rect', { rect }),
    rehover: () => invoke('rehover'),
    onHover: (cb) => on('hover', cb),
    onRefresh: noop,
    onLiveState: (cb) => {
      on('live:state', cb, (e) => e.payload.summary);
      invoke('live_state').then((p) => cb(p.summary));
    },
    onAlert: noop,
    onToggle: noop,
    onSummon: noop,
    onBig: noop,
    onBigAnchor: noop,
    onBigGeometry: noop,
    reportPinned: noop,
    onSetPinned: noop,
    onSettings: noop,
    updateState: () => Promise.resolve({ status: 'idle', version: null, url: null, checkedAt: 0 }),
    onUpdate: noop,
    updateAct: noop,
    analysis: {
      sessions: unsupported('Analysis'),
      get: unsupported('Analysis'),
      pick: unsupported('Analysis'),
      telemetry: unsupported('Telemetry'),
      choose: unsupported('Menu'),
      location: unsupported('Location'),
      replayInfo: unsupported('Replay'),
      replayChunk: unsupported('Replay'),
    },
  };
})();
