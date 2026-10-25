// Is the Live tab the main one? It is from when the app connects (15 minutes
// before a session) until a while after the session finishes. After that it
// stays available, at the end of the tabs, but the island no longer opens on it.
(function (root) {
  const RUNNING = new Set(['Started', 'Aborted']);
  const FINISHED = new Set(['Finished', 'Finalised', 'Ends']);

  class LiveTab {
    constructor(afterMs = 15 * 60e3) {
      this.afterMs = afterMs;
      this.reset();
    }

    reset() {
      this.sawRunning = false;
      this.finishedAt = null;
    }

    // live: the live timing state (or null once disconnected). scheduledEnd:
    // when the session was due to end, used if the app started after it
    // finished and never saw it run.
    update(live, now = Date.now(), scheduledEnd = null) {
      if (!live) return this.reset();
      const status = live.status;
      if (RUNNING.has(status)) {
        this.sawRunning = true;
        this.finishedAt = null;
      } else if (FINISHED.has(status)) {
        if (this.finishedAt == null) this.finishedAt = this.sawRunning ? now : scheduledEnd ?? 0;
      } else if (status === 'Inactive') {
        this.reset(); // the next session, not started yet
      }
    }

    isMain(connected, now = Date.now()) {
      if (!connected) return false;
      return this.finishedAt == null || now - this.finishedAt <= this.afterMs;
    }
  }

  const api = { LiveTab };
  if (typeof module !== 'undefined') module.exports = api;
  else root.LiveTab = LiveTab;
})(this);
