/* Retrieval Probe view: run the real FTS engine for a query and show — and let you
 * copy — exactly what `/api/search` returns. This is the "do I trust retrieval?" view:
 * no rewriting, no re-ranking on the client; it renders the engine's own output. */
"use strict";
(function () {
  const { api, el, div, preview, fmtTime, badge } = YM;
  const PAGE_LIMIT = 20;

  function mount(container) {
    const wrap = div("probe");

    // Query bar ──────────────────────────────────────────────────────────────
    const bar = div("probe-bar");
    const input = el("input", "token-input probe-input");
    input.type = "search";
    input.placeholder = "Search memories (FTS) — e.g. deploy rollback";
    input.spellcheck = false;
    input.autocomplete = "off";
    const run = el("button", "btn", "Search");
    bar.append(input, run);
    wrap.appendChild(bar);

    const meta = div("probe-meta");
    wrap.appendChild(meta);

    const results = div("probe-results");
    results.appendChild(div("tree-empty", "Type a query and hit Search to probe the retrieval engine."));
    wrap.appendChild(results);

    container.appendChild(wrap);

    let lastHits = [];
    async function search() {
      const q = input.value.trim();
      if (!q) {
        results.innerHTML = "";
        results.appendChild(div("tree-empty", "Type a query and hit Search to probe the retrieval engine."));
        meta.innerHTML = "";
        lastHits = [];
        return;
      }
      results.innerHTML = "";
      results.appendChild(div("tree-loading", "Running FTS…"));
      meta.innerHTML = "";
      const hits = await api(`/api/search?q=${encodeURIComponent(q)}&limit=${PAGE_LIMIT}`);
      lastHits = hits;
      renderMeta(meta, q, hits);
      results.innerHTML = "";
      if (!hits.length) {
        results.appendChild(div("tree-empty", `No matches for “${q}”. The engine returned nothing — this is what your agent would retrieve.`));
        return;
      }
      hits.forEach((h, i) => results.appendChild(hitRow(h, i)));
    }

    function renderMeta(node, q, hits) {
      node.innerHTML = "";
      const n = hits.length;
      const summary = div("probe-summary", `${n} result${n === 1 ? "" : "s"} for “${q}”${n === PAGE_LIMIT ? " (capped)" : ""}`);
      const copyAll = el("button", "btn btn--ghost probe-copy", "Copy JSON");
      copyAll.title = "Copy the raw retrieval payload (exactly what the engine returns)";
      copyAll.addEventListener("click", () => copy(copyAll, JSON.stringify(lastHits, null, 2)));
      node.append(summary, copyAll);
    }

    run.addEventListener("click", () => search().catch(YM.handleError));
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") run.click(); });

    return Promise.resolve();
  }

  function hitRow(h, i) {
    const d = h.drawer || {};
    const row = div("probe-hit" + (d.is_invalidated ? " invalid" : ""));

    const head = div("probe-hit-head");
    head.append(
      div("probe-rank", "#" + (i + 1)),
      badge(d.confidence),
      div("probe-path", `${h.wing} / ${h.room}`),
      div("probe-score", "rank " + fmtRank(h.rank)),
    );
    row.appendChild(head);

    row.appendChild(div("probe-content", preview(d.content, 280)));

    const foot = div("probe-hit-foot");
    foot.append(
      div("probe-dim", "#" + d.id),
      div("probe-dim", d.source || "—"),
      div("probe-dim", "accessed " + fmtTime(d.last_accessed_at)),
    );
    const copyOne = el("button", "btn btn--ghost probe-copy", "Copy");
    copyOne.title = "Copy this memory's content";
    copyOne.addEventListener("click", () => copy(copyOne, d.content || ""));
    foot.appendChild(copyOne);
    row.appendChild(foot);

    return row;
  }

  function fmtRank(r) { return typeof r === "number" ? r.toFixed(3) : String(r); }

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

  YM.register("probe", { title: "Retrieval Probe", mount });
})();
