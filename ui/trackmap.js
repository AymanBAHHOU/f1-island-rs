// Fits track coordinates (OpenF1 x/y) into an SVG box. Shared by the island's
// Circuit page and the analysis view.
window.TrackMap = (() => {
  // Rotate the track to the angle that fills the box best, then scale and
  // centre it. Returns project(x, y) -> [svgX, svgY].
  function fit(points, width, height, pad = 10) {
    let best = null;
    for (let deg = 0; deg < 180; deg += 5) {
      const a = (deg * Math.PI) / 180;
      const cos = Math.cos(a);
      const sin = Math.sin(a);
      let minX = Infinity;
      let maxX = -Infinity;
      let minY = Infinity;
      let maxY = -Infinity;
      for (const p of points) {
        const x = p.x * cos - p.y * sin;
        const y = p.x * sin + p.y * cos;
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
      const scale = Math.min((width - 2 * pad) / (maxX - minX || 1), (height - 2 * pad) / (maxY - minY || 1));
      if (!best || scale > best.scale) best = { cos, sin, scale, minX, maxX, minY, maxY };
    }
    const { cos, sin, scale, minX, maxX, minY, maxY } = best;
    const offX = (width - (maxX - minX) * scale) / 2;
    const offY = (height - (maxY - minY) * scale) / 2;
    return (x, y) => {
      const rx = x * cos - y * sin;
      const ry = x * sin + y * cos;
      // SVG y grows downwards; flip so the map isn't mirrored.
      return [offX + (rx - minX) * scale, height - (offY + (ry - minY) * scale)];
    };
  }

  const pathOf = (points, project, close = true) =>
    points
      .map((p, i) => {
        const [x, y] = project(p.x, p.y);
        return `${i ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join('') + (close ? 'Z' : '');

  // Plain outline with a start/finish marker.
  function outline(points, width, height, { pad = 10, stroke = 5 } = {}) {
    const project = fit(points, width, height, pad);
    const [sx, sy] = project(points[0].x, points[0].y);
    return `<svg class="track-map" viewBox="0 0 ${width} ${height}" aria-label="Track map">
      <path d="${pathOf(points, project)}" fill="none" stroke="rgba(255,255,255,0.18)" stroke-width="${stroke + 4}" stroke-linejoin="round" stroke-linecap="round"/>
      <path d="${pathOf(points, project)}" fill="none" stroke="#e5e5ea" stroke-width="${stroke - 2}" stroke-linejoin="round" stroke-linecap="round"/>
      <circle cx="${sx.toFixed(1)}" cy="${sy.toFixed(1)}" r="${stroke}" fill="#e10600" stroke="#000" stroke-width="1.5"><title>Start / finish</title></circle>
    </svg>`;
  }

  return { fit, pathOf, outline };
})();
