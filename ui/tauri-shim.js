// Implements the `window.f1` interface the page expects (in the Electron app, preload.js
// provides it) on top of Tauri's `invoke` (page -> Rust) and `listen` (Rust -> page).
// Features not yet ported reject or do nothing, so the page degrades instead of breaking.
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  const on = (name, cb, pick = (e) => e.payload) => {
    listen(name, (e) => cb(pick(e)));
  };
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
    notch: () => invoke('get_notch'),
    onNotch: (cb) => on('island:notch', cb),
    demo: () => invoke('get_demo'),
    onDemo: (cb) => on('island:demo', cb),
    setInteractive: (v) => invoke('set_interactive', { on: !!v }),
    dragStart: () => invoke('drag_start'),
    dragEnd: () => invoke('drag_end'),
    openMenu: () => invoke('open_menu'),
    setBig: (on) => invoke('set_big', { on: !!on }),
    setRect: (rect) => invoke('report_rect', { rect }),
    rehover: () => invoke('rehover'),
    onHover: (cb) => on('hover', cb),
    onRefresh: (cb) => on('f1:refresh', () => cb()),
    onLiveState: (cb) => {
      on('live:state', cb, (e) => e.payload.summary);
      invoke('live_state').then((p) => cb(p.summary));
    },
    onAlert: (cb) => on('live:alert', cb),
    onToggle: (cb) => on('island:toggle', () => cb()),
    onSummon: (cb) => on('island:summon', () => cb()),
    onBig: (cb) => on('island:big', () => cb()),
    onBigAnchor: (cb) => on('island:big-anchor', cb),
    onBigGeometry: (cb) => on('island:big-geometry', cb),
    reportPinned: (v) => invoke('report_pinned', { on: !!v }),
    onSetPinned: (cb) => on('island:set-pinned', cb),
    onSettings: (cb) => on('island:settings', cb),
    updateState: () => invoke('update_state'),
    onUpdate: (cb) => on('update:state', cb),
    updateAct: () => invoke('update_act'),
    analysis: {
      sessions: () => invoke('analysis_sessions'),
      get: (key) => invoke('analysis_get', { key }),
      pick: (key) => invoke('analysis_pick', { currentKey: key ?? null }),
      telemetry: (sessionKey, driver, lap) =>
        invoke('analysis_telemetry', { sessionKey, driver, lap }),
      choose: (items) => invoke('choose', { items }),
      location: (sessionKey, driver, lap) =>
        invoke('analysis_location', { sessionKey, driver, lap }),
      replayInfo: (sessionKey) => invoke('replay_info', { sessionKey }),
      replayChunk: (sessionKey, index) => invoke('replay_chunk', { sessionKey, index }),
    },
  };
})();
