/* Admin view: palace stats + access-token management. Requires a global admin grant. */
"use strict";
(function () {
  const { api, div, fmtTime } = YM;

  async function mount(container) {
    const statsSec = div("admin-section");
    statsSec.appendChild(div("admin-h", "Palace"));
    container.appendChild(statsSec);
    try {
      renderStats(statsSec, await api("/api/stats"));
    } catch (e) {
      statsSec.appendChild(notice(e));
    }

    const tokSec = div("admin-section");
    tokSec.appendChild(div("admin-h", "Access tokens"));
    container.appendChild(tokSec);
    try {
      await renderTokens(tokSec);
    } catch (e) {
      tokSec.appendChild(notice(e));
    }
  }

  function notice(e) {
    if (e && e.status === 403)
      return div("pane-msg", "This view needs a global admin token (a grant of *:admin). Connect with one above.");
    if (e && e.status === 401)
      return div("pane-msg", "Authentication required — connect with an admin token above.");
    return div("pane-msg", "Error: " + (e && e.message ? e.message : e));
  }

  function renderStats(sec, s) {
    const grid = div("stat-grid");
    const cards = [
      ["Drawers", s.total_drawers],
      ["Relations", s.total_relations],
      ["Wings", s.total_wings],
      ["DB size", fmtBytes(s.db_size_bytes)],
      ["Last compact", s.last_compact ? fmtTime(s.last_compact) : "never"],
    ];
    for (const [k, v] of cards) {
      const card = div("stat-card");
      card.append(div("stat-val", String(v)), div("stat-key", k));
      grid.appendChild(card);
    }
    sec.appendChild(grid);
  }

  async function renderTokens(sec) {
    const list = div("token-list");
    sec.appendChild(list);
    sec.appendChild(createForm(() => reload(list)));
    await reload(list);
  }

  async function reload(list) {
    list.innerHTML = "";
    list.appendChild(div("tree-loading", "Loading tokens…"));
    const tokens = await api("/api/tokens");
    list.innerHTML = "";
    if (!tokens.length) { list.appendChild(div("tree-empty", "No tokens yet.")); return; }

    const table = document.createElement("table");
    table.className = "data-table";
    table.innerHTML = "<thead><tr><th>ID</th><th>Label</th><th>Grants</th><th>Last used</th><th>Status</th><th></th></tr></thead>";
    const tbody = document.createElement("tbody");
    for (const t of tokens) {
      const tr = document.createElement("tr");
      if (t.revoked) tr.className = "revoked";
      const grants = t.grants.map((g) => `${g.wing}:${g.level}`).join(", ") || "—";
      tr.append(td("#" + t.id), td(t.label), td(grants, "mono"), td(t.last_used_at ? fmtTime(t.last_used_at) : "—"),
        td(t.revoked ? "revoked" : "active", t.revoked ? "op-revoke" : "op-insert"));
      const actionCell = document.createElement("td");
      if (!t.revoked) {
        const b = document.createElement("button");
        b.className = "btn btn--danger"; b.textContent = "Revoke";
        b.addEventListener("click", async () => {
          if (!confirm(`Revoke token #${t.id} (${t.label})?`)) return;
          try { await api(`/api/tokens/${t.id}`, { method: "DELETE" }); await reload(list); }
          catch (e) { YM.handleError(e); }
        });
        actionCell.appendChild(b);
      }
      tr.appendChild(actionCell);
      tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    list.appendChild(table);
  }

  function createForm(onCreated) {
    const form = div("create-form");
    form.appendChild(div("admin-h2", "Create token"));
    const label = input("label, e.g. ci");
    const grants = input("grants, e.g. engineering:read, legal:write");
    const submit = document.createElement("button");
    submit.className = "btn"; submit.textContent = "Create";
    const out = div("secret-out");
    out.hidden = true;

    const row = div("form-row");
    row.append(labeled("Label", label), labeled("Grants", grants), submit);
    form.append(row, out);

    submit.addEventListener("click", async () => {
      const grantList = grants.value.split(",").map((g) => g.trim()).filter(Boolean);
      if (!label.value.trim() || !grantList.length) {
        YM.showBanner("warn", "A label and at least one grant (wing:level) are required.");
        return;
      }
      try {
        const res = await api("/api/tokens", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ label: label.value.trim(), grants: grantList }),
        });
        out.hidden = false;
        out.innerHTML = "";
        out.append(
          div("secret-label", `Token #${res.id} created — copy the secret now, it won't be shown again:`),
          Object.assign(div("secret-value"), { textContent: res.secret }),
        );
        label.value = ""; grants.value = "";
        onCreated();
      } catch (e) { YM.handleError(e); }
    });
    return form;
  }

  // helpers
  function td(text, cls) { const c = document.createElement("td"); if (cls) c.className = cls; c.textContent = text; return c; }
  function input(ph) { const i = document.createElement("input"); i.className = "filter-input"; i.placeholder = ph; return i; }
  function labeled(label, el) { const f = div("filter-field"); f.append(div("filter-label", label), el); return f; }
  function fmtBytes(n) {
    if (!n) return "0 B";
    const u = ["B", "KB", "MB", "GB"]; let i = 0; let v = n;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return v.toFixed(i ? 1 : 0) + " " + u[i];
  }

  YM.register("admin", { title: "Admin", mount });
})();
