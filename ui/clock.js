// Session clock: the live feed sends "Remaining" at a UTC time and whether
// it's running; a running clock is counted down locally.
(function (root) {
  function remainingMs(clock, now = Date.now()) {
    if (!clock?.remaining) return null;
    const [h = 0, m = 0, s = 0] = String(clock.remaining).split(':').map(Number);
    const at = (h * 3600 + m * 60 + s) * 1000;
    if (!clock.running) return at;
    return Math.max(0, at - Math.max(0, now - Date.parse(clock.utc)));
  }

  // "1:03:05" / "12:07"
  function formatClock(ms) {
    const total = Math.round(ms / 1000);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = String(total % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
  }

  const api = { remainingMs, formatClock };
  if (typeof module !== 'undefined') module.exports = api;
  else root.SessionClock = api;
})(this);
