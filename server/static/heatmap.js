/* Confidence Heatmap: one cell per room, grouped by wing.
 * Color = confidence × recency (green healthy → amber aging → red stale/low).
 * Cell size ∝ drawer count. Click a cell to jump to the Room Navigator.
 */
"use strict";
(function () {
  const { api, div, fmtTime } = YM;

  async function mount(container) {
    const stats = await api("/api/heatmap");
    if (!stats.length) {
      container.appendChild(div("pane-msg", "No rooms visible for this token."));
      return;
    }
    const maxDrawers = Math.max(1, ...stats.map((s) => s.drawer_count));

    // Group rooms by wing, preserving order.
    const byWing = new Map();
    for (const s of stats) {
      if (!byWing.has(s.wing)) byWing.set(s.wing, []);
      byWing.get(s.wing).push(s);
    }

    const legend = div("heatmap-legend");
    legend.innerHTML =
      '<span><i class="hk" style="background:#3fb950"></i>healthy</span>' +
      '<span><i class="hk" style="background:#d29922"></i>aging</span>' +
      '<span><i class="hk" style="background:#f85149"></i>stale / low confidence</span>' +
      '<span class="hk-note">cell size ∝ drawer count</span>';
    container.appendChild(legend);

    const grid = div("heatmap-grid");
    for (const [wing, rooms] of byWing) {
      const col = div("heatmap-col");
      col.appendChild(div("heatmap-wing", wing));
      for (const s of rooms) col.appendChild(cell(s, maxDrawers));
      grid.appendChild(col);
    }
    container.appendChild(grid);
  }

  function cell(s, maxDrawers) {
    const score = healthScore(s);
    const size = 38 + Math.round(46 * Math.sqrt(s.drawer_count / maxDrawers));
    const c = div("heatmap-cell");
    c.style.width = size + "px";
    c.style.height = size + "px";
    c.style.background = colorFor(score);
    c.style.opacity = String(0.5 + 0.5 * score);
    c.appendChild(div("heatmap-cell-label", s.room));
    if (s.drawer_count) c.appendChild(div("heatmap-cell-count", String(s.drawer_count)));
    c.title =
      `${s.wing} / ${s.room}\n` +
      `drawers: ${s.drawer_count}\n` +
      `avg confidence: ${(s.avg_confidence * 100).toFixed(0)}%\n` +
      `last write: ${fmtTime(s.last_write)}\n` +
      `last read: ${fmtTime(s.last_read)}`;
    c.addEventListener("click", () => { location.hash = "#rooms"; });
    return c;
  }

  function daysSince(iso) {
    if (!iso) return Infinity;
    const t = new Date(iso).getTime();
    if (isNaN(t)) return Infinity;
    return (Date.now() - t) / 86400000;
  }

  /** Combine confidence with recency into a [0,1] health score. */
  function healthScore(s) {
    if (!s.drawer_count) return 0;
    const recent = Math.min(daysSince(s.last_read), daysSince(s.last_write));
    const recencyFactor = recent < 7 ? 1 : recent < 30 ? 0.7 : 0.4;
    return Math.max(0, Math.min(1, s.avg_confidence * recencyFactor));
  }

  function colorFor(score) {
    if (score >= 0.66) return "#3fb950";
    if (score >= 0.4) return "#d29922";
    return "#f85149";
  }

  YM.register("heatmap", { title: "Confidence Heatmap", mount });
})();
