// A single interpolated geometry drives the body, corners and shoulders.
// Exponential interpolation is frame-rate independent and never overshoots;
// interrupted animations continue from their current shape, not a keyframe.
(() => {
  const lerp = (from, to, amount) => from + (to - from) * amount;
  function interpolate(from, to, seconds) {
    const amount = 1 - Math.exp(-16 * Math.max(0, seconds));
    return Object.fromEntries(Object.keys(to).map((key) => [key, lerp(from[key], to[key], amount)]));
  }

  function surfacePath({ width: w, height: h, radius, shoulder: s }, pad = 32) {
    const r = Math.min(radius, w / 2, Math.max(0, (h - s) / 2));
    const k = 0.5522847498; // cubic approximation of a quarter circle
    const l = pad;
    const right = l + w;
    // Each curve meets its neighbouring horizontal/vertical edge tangentially.
    return [
      `M ${l - s} 0 H ${right + s}`,
      `C ${right + s * (1 - k)} 0 ${right} ${s * (1 - k)} ${right} ${s}`,
      `V ${h - r} C ${right} ${h - r + r * k} ${right - r + r * k} ${h} ${right - r} ${h}`,
      `H ${l + r} C ${l + r - r * k} ${h} ${l} ${h - r + r * k} ${l} ${h - r}`,
      `V ${s} C ${l} ${s * (1 - k)} ${l - s * (1 - k)} 0 ${l - s} 0 Z`,
    ].join(' ');
  }

  function create(element, surface, initial, reduced = false) {
    let current = { ...initial };
    let target = { ...initial };
    let frame = null;
    let previous = null;
    function draw() {
      element.style.width = `${current.width}px`;
      element.style.height = `${current.height}px`;
      element.style.borderRadius = `0 0 ${current.radius}px ${current.radius}px`;
      surface.setAttribute('width', current.width + 64);
      surface.setAttribute('height', current.height + 32);
      surface.firstElementChild.setAttribute('d', surfacePath(current));
    }
    function tick(time) {
      frame = null;
      current = interpolate(current, target, previous == null ? 0 : (time - previous) / 1000);
      previous = time;
      const done = Object.keys(target).every((key) => Math.abs(current[key] - target[key]) < 0.05);
      if (done) current = { ...target };
      draw();
      if (!done) frame = requestAnimationFrame(tick);
      else previous = null;
    }
    draw();
    return {
      move(next) {
        if (Object.keys(next).every((key) => target[key] === next[key])) return;
        target = { ...next };
        if (reduced) {
          cancelAnimationFrame(frame);
          frame = previous = null;
          current = { ...target };
          draw();
        } else if (frame == null) {
          frame = requestAnimationFrame(tick);
        }
      },
      reduce(on) {
        reduced = on;
        if (on) {
          cancelAnimationFrame(frame);
          frame = previous = null;
          current = { ...target };
          draw();
        }
      },
      stop() {
        cancelAnimationFrame(frame);
        element.style.width = element.style.height = element.style.borderRadius = '';
      },
    };
  }

  const api = { lerp, interpolate, surfacePath, create };
  if (typeof module !== 'undefined') module.exports = api;
  else window.NotchMotion = api;
})();
