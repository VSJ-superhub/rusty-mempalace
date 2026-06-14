/* Session Diff view: reconstructs write *sessions* from the audit trail (bursts of
 * change separated by idle gaps) and shows what each one touched — the "what changed
 * since last time I worked in here?" view. No snapshot storage: a session is just a
 * window of `/api/sessions`, rolled up to op/wing counts plus the actual changes. */
"use strict";
(function () {
  const { api, el, div, preview, fmtTime, handleError } = YM;
  const DEFAULT_GAP = 30; // minutes of idle that splits one session from the next

  function mount(container) {
    const wrap = div("sessions");

    // Controls ────────────────────────────────────────────────────────────────
    const bar = div("sessions-bar");
    bar.append(el("span", "sessions-label", "Idle gap (min)"));
    const gapInput = el("input", "token-input sessions-gap");
    gapInput.type = "number";
    gapInput.min = "1";
    gapInput.max = "1440";
    gapInput.value = String(DEFAULT_GAP);
    gapInput.title = "Minutes of inactivity that separate one write session from the next";
    const reload = el("button", "btn", "Apply");
    bar.append(gapInput, reload);
    wrap.appendChild(bar);

    const meta = div("sessions-meta");
    wrap.appendChild(meta);

    const list = div("sessions-list");
    list.appendChild(div("tree-loading", "Loading sessions…"));
    wrap.appendChild(list);

    container.appendChild(wrap);

    async function load() {
      const gap = Math.min(1440, Math.max(1, parseInt(gapInput.value, 10) || DEFAULT_GAP));
      list.innerHTML = "";
      list.appendChild(div("tree-loading", "Reconstructing sessions from the audit trail…"));
      meta.innerHTML = "";
      const sessions = await api(`/api/sessions?gap=${gap}&limit=50`);
      renderMeta(meta, sessions, gap);
      list.innerHTML = "";
      if (!sessions.length) {
        list.appendChild(div("tree-empty", "No write sessions yet — this view fills in as memories are created, updated, or invalidated."));
        return;
      }
      sessions.forEach((s, i) => list.appendChild(sessionCard(s, i, sessions.length)));
    }

    function renderMeta(node, sessions, gap) {
      node.innerHTML = "";
      const n = sessions.length;
      const total = sessions.reduce((a, s) => a + (s.total_changes || 0), 0);
      node.append(
        div("sessions-summary", `${n} session${n === 1 ? "" : "s"} · ${total} change${total === 1 ? "" : "s"} · split at ${gap} min idle`),
      );
      const copyAll = el("button", "btn btn--ghost", "Copy JSON");
      copyAll.title = "Copy the raw session payload";
      copyAll.addEventListener("click", () => copy(copyAll, JSON.stringify(sessions, null, 2)));
      node.append(copyAll);
    }

    reload.addEventListener("click", () => load().catch(handleError));
    gapInput.addEventListener("keydown", (e) => { if (e.key === "Enter") reload.click(); });

    return load();
  }

  // newest session is index 0; label them "latest", then "−1", "−2"…
  function sessionCard(s, i, total) {
    const card = div("session-card");

    const head = div("session-head");
    const label = i === 0 ? "latest" : "−" + i;
    head.append(
      div("session-tag", label),
      div("session-range", `${fmtTime(s.started_at)} → ${fmtTime(s.ended_at)}`),
      div("session-dur", fmtSpan(s.started_at, s.ended_at)),
      div("session-count", `${s.total_changes} change${s.total_changes === 1 ? "" : "s"}`),
    );
    card.appendChild(head);

    // Roll-up chips: operations and wings touched.
    const rollup = div("session-rollup");
    (s.op_counts || []).forEach((c) => rollup.appendChild(chip("op", c.label, c.count)));
    (s.wing_counts || []).forEach((c) => rollup.appendChild(chip("wing", c.label, c.count)));
    card.appendChild(rollup);

    // The diff itself: collapsible list of the actual changes in this window.
    const details = el("details", "session-details");
    const summary = el("summary", "session-details-sum",
      `Show ${s.entries ? s.entries.length : 0} change${(s.entries && s.entries.length === 1) ? "" : "s"}${s.truncated ? " (sampled)" : ""}`);
    details.appendChild(summary);
    const rows = div("session-changes");
    (s.entries || []).forEach((e) => rows.appendChild(changeRow(e)));
    details.appendChild(rows);
    card.appendChild(details);

    return card;
  }

  function chip(kind, label, count) {
    const c = div("session-chip session-chip--" + kind);
    c.append(el("span", "session-chip-label", label), el("span", "session-chip-count", String(count)));
    return c;
  }

  function changeRow(e) {
    const row = div("session-change op-" + (e.operation || "").toLowerCase());
    row.append(
      div("session-op", e.operation || "?"),
      div("session-wing", e.wing || "—"),
      div("session-preview", preview(e.preview, 100)),
      div("session-time", fmtTime(e.created_at)),
    );
    return row;
  }

  // Human duration between two RFC3339 timestamps.
  function fmtSpan(a, b) {
    const ms = new Date(b) - new Date(a);
    if (!isFinite(ms) || ms < 0) return "—";
    const m = Math.round(ms / 60000);
    if (m < 1) return "<1 min";
    if (m < 60) return m + " min";
    const h = Math.floor(m / 60), rem = m % 60;
    return rem ? `${h}h ${rem}m` : `${h}h`;
  }

  function copy(btn, text) {
    const done = () => { const o = btn.textContent; btn.textContent = "Copied"; setTimeout(() => (btn.textContent = o), 1200); };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(done, () => fallbackCopy(text, done));
    } else { fallbackCopy(text, done); }
  }
  function fallbackCopy(text, done) {
    const ta = document.createElement("textarea");
    ta.value = text; ta.style.position = "fixed"; ta.style.opacity = "0";
    document.body.appendChild(ta); ta.select();
    try { document.execCommand("copy"); done(); } catch (_) {}
    document.body.removeChild(ta);
  }

  YM.register("sessions", { title: "Session Diff", mount });
})();
