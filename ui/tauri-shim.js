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
    trackMap: () => invoke('track_map'),
    weekendResults: () => invoke('weekend_results'),
    openArticle: (url) => invoke('open_article', { url }),
    settings: () => invoke('get_settings'),
    notch: () => Promise.resolve(null),
    onNotch: noop,
    demo: () => invoke('get_demo'),
    onDemo: (cb) => on('island:demo', cb),
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
    onAlert: (cb) => on('live:alert', cb),
    onToggle: noop,
    onSummon: noop,
    onBig: noop,
    onBigAnchor: noop,
    onBigGeometry: noop,
    reportPinned: noop,
    onSetPinned: noop,
    onSettings: (cb) => on('island:settings', cb),
    updateState: () => invoke('update_state'),
    onUpdate: (cb) => on('update:state', cb),
    updateAct: () => invoke('update_act'),
    analysis: {
      sessions: () => invoke('analysis_sessions'),
      get: (key) => invoke('analysis_get', { key }),
      pick: unsupported('Analysis'),
      telemetry: (sessionKey, driver, lap) =>
        invoke('analysis_telemetry', { sessionKey, driver, lap }),
      choose: unsupported('Menu'),
      location: (sessionKey, driver, lap) =>
        invoke('analysis_location', { sessionKey, driver, lap }),
      replayInfo: (sessionKey) => invoke('replay_info', { sessionKey }),
      replayChunk: (sessionKey, index) => invoke('replay_chunk', { sessionKey, index }),
    },
  };
})();
