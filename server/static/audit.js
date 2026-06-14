/* Audit Log view: filterable WAL table + CSV export. */
"use strict";
(function () {
  const { api, div, fmtTime } = YM;

  function mount(container) {
    const bar = div("filter-bar");
    const op = sel("op", ["", "INSERT", "UPDATE", "INVALIDATE", "COMPACT", "GRANT", "REVOKE"]);
    const wing = inp("wing", "wing name");
    const from = inp("from", "from (ISO date)");
    const to = inp("to", "to (ISO date)");
    const apply = button("Apply", "btn");
    const csv = button("Export CSV", "btn btn--ghost");
    bar.append(field("Operation", op), field("Wing", wing), field("From", from), field("To", to), apply, csv);

    const tableWrap = div("table-wrap");
    container.append(bar, tableWrap);

    function query() {
      const p = new URLSearchParams();
      if (op.value) p.set("op", op.value);
      if (wing.value.trim()) p.set("wing", wing.value.trim());
      if (from.value.trim()) p.set("from", from.value.trim());
      if (to.value.trim()) p.set("to", to.value.trim());
      return p.toString();
    }

    async function load() {
      tableWrap.innerHTML = "";
      tableWrap.appendChild(div("tree-loading", "Loading audit log…"));
      const entries = await api("/api/audit" + (query() ? "?" + query() : ""));
      tableWrap.innerHTML = "";
      tableWrap.appendChild(renderTable(entries));
    }

    apply.addEventListener("click", load);
    csv.addEventListener("click", async () => {
      try {
        const text = await api("/api/audit/export" + (query() ? "?" + query() : ""));
        const blob = new Blob([text], { type: "text/csv" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url; a.download = "audit.csv"; a.click();
        URL.revokeObjectURL(url);
      } catch (e) { YM.handleError(e); }
    });

    return load();
  }

  function renderTable(entries) {
    if (!entries.length) return div("tree-empty", "No audit entries for these filters.");
    const table = document.createElement("table");
    table.className = "data-table";
    table.innerHTML =
      "<thead><tr><th>Timestamp</th><th>Operation</th><th>Table</th><th>Record</th><th>Wing</th><th>Preview</th></tr></thead>";
    const tbody = document.createElement("tbody");
    for (const e of entries) {
      const tr = document.createElement("tr");
      tr.append(
        td(fmtTime(e.created_at)),
        td(e.operation, "op-" + e.operation.toLowerCase()),
        td(e.table_name),
        td("#" + e.record_id),
        td(e.wing || "—"),
        td(e.preview, "mono"),
      );
      tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    return table;
  }

  // ── tiny form helpers ──────────────────────────────────────────────────────
  function td(text, cls) { const c = document.createElement("td"); if (cls) c.className = cls; c.textContent = text; return c; }
  function inp(name, ph) { const i = document.createElement("input"); i.className = "filter-input"; i.placeholder = ph; i.name = name; return i; }
  function sel(name, opts) {
    const s = document.createElement("select"); s.className = "filter-input"; s.name = name;
    for (const o of opts) { const opt = document.createElement("option"); opt.value = o; opt.textContent = o || "(any)"; s.appendChild(opt); }
    return s;
  }
  function button(text, cls) { const b = document.createElement("button"); b.className = cls; b.textContent = text; return b; }
  function field(label, input) { const f = div("filter-field"); f.append(div("filter-label", label), input); return f; }

  YM.register("audit", { title: "Audit Log", mount });
})();
