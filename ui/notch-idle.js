// Pure presentation model shared by the actual bar and deterministic tests.
(() => {
  const FLAGS = {
    2: ['yellow', 'YELLOW FLAG'], 4: ['sc', 'SAFETY CAR'], 5: ['red', 'RED FLAG'],
    6: ['vsc', 'VIRTUAL SC'], 7: ['vsc', 'VSC ENDING'],
  };
  const countdown = (ms) => {
    const s = Math.max(0, Math.floor(ms / 1000));
    const pad = (n) => String(n).padStart(2, '0');
    if (s >= 86400) return `${Math.floor(s / 86400)}d ${pad(Math.floor(s / 3600) % 24)}h`;
    return `${pad(Math.floor(s / 3600))}:${pad(Math.floor(s / 60) % 60)}:${pad(s % 60)}`;
  };
  const events = (live) => (live?.messages || [])
    .filter((m) => typeof m.message === 'string' && m.message.trim() &&
      !/^CLEAR IN TRACK SECTOR|\bDELETED\b/i.test(m.message))
    .slice(0, 5);

  function model({ state, snapshot, live, now, eventIndex = 0 }) {
    const demo = !!snapshot?.demo;
    const prefix = demo ? 'DEMO · ' : '';
    const name = snapshot?.weekend?.name?.replace(/^DEMO · /, '').replace(/Grand Prix/i, 'GP') || 'Formula 1';
    const base = { demo, kind: state.kind, tone: 'red', live: false, kicker: `${prefix}NEXT UP`,
      title: name, status: 'ON THE GRID', value: 'Loading…', chip: 'NEXT', detail: 'Fetching the weekend schedule', eventKey: '' };
    if (state.kind === 'next') {
      const start = Date.parse(state.session.start);
      return { ...base, status: `${state.session.short} · STARTS IN`,
        value: Number.isFinite(start) ? countdown(start - now) : 'TBA',
        detail: `${state.session.name} · ${Number.isFinite(start) ? new Date(start).toLocaleString(undefined, {
          weekday: 'short', hour: '2-digit', minute: '2-digit',
        }) : 'Time to be confirmed'}` };
    }
    if (state.kind === 'live') {
      const delayed = live?.status === 'Inactive';
      const finished = ['Finished', 'Finalised', 'Ends'].includes(live?.status);
      const flag = !finished && FLAGS[live?.track?.status];
      const clear = String(live?.track?.status) === '1';
      const messages = events(live);
      const event = messages[Math.max(0, Math.floor(eventIndex)) % (messages.length || 1)];
      const eventFlag = String(event?.flag || '').toUpperCase();
      const eventTone = eventFlag === 'RED' ? 'red' : eventFlag.includes('YELLOW') ? 'yellow'
        : event?.category === 'SafetyCar' ? 'sc' : event ? 'info' : 'green';
      const leader = live?.top?.[0]?.code;
      const tone = finished ? 'neutral' : delayed ? 'info' : flag ? flag[0] : event ? eventTone : clear ? 'green' : 'neutral';
      return { ...base, live: !finished, tone,
        kicker: `${prefix}${finished ? 'FINISHED' : demo ? 'LIVE' : 'LIVE ' + (state.session.short || 'SESSION')}`,
        title: live?.lap ? `LAP ${live.lap}${live.totalLaps ? ` / ${live.totalLaps}` : ''}` : name,
        status: finished ? 'CHEQUERED FLAG' : delayed ? 'START DELAYED' : flag ? flag[1] : clear ? 'TRACK CLEAR' : 'CONNECTING',
        value: leader ? `${leader} · P1` : delayed ? 'Stand by' : 'Live timing',
        chip: finished ? 'FINISH' : flag ? { sc: 'SC', vsc: 'VSC', red: 'RED', yellow: 'YEL' }[flag[0]] : 'LIVE',
        detail: finished ? 'Session complete · hover for the weekend' : event?.message ||
          (delayed ? 'Waiting for the announced start' : flag ? flag[1]
            : clear ? 'Track is green · waiting for race control' : 'Waiting for the live timing feed'),
        eventKey: event ? `${event.utc || ''}:${event.message}` : '',
      };
    }
    if (state.kind === 'error') return { ...base, tone: 'neutral', kicker: `${prefix}F1 ISLAND`, title: 'Offline',
      status: 'DATA CONNECTION', value: 'Retrying…', chip: 'OFFLINE', detail: 'Schedule unavailable · reconnecting automatically' };
    if (state.kind === 'finished') return { ...base, tone: 'neutral', kicker: `${prefix}WEEKEND COMPLETE`,
      status: 'CHEQUERED FLAG', value: 'Finished', chip: 'FINISH', detail: 'Hover for results, standings and session analysis' };
    return { ...base, tone: 'neutral', kicker: `${prefix}F1 ISLAND`, title: snapshot ? 'Season break' : 'Getting ready',
      status: 'SCHEDULE', value: snapshot ? 'Stand by' : 'Loading…', chip: 'F1',
      detail: snapshot ? 'The next session will appear here' : base.detail };
  }

  const api = { model, events, countdown };
  if (typeof module !== 'undefined') module.exports = api;
  else window.NotchIdle = api;
})();
