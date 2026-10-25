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

  // The weekend, standings, title fight and circuit history, all put together in Rust.
  const snapshot = (opts = {}) => invoke('snapshot', { extras: opts.extras !== false });

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
