/* yourmemory dashboard — core + router.
 * Exposes window.YM with a fetch wrapper, shared helpers, and a tiny hash router.
 * View modules (rooms/graph/audit/heatmap/admin) call YM.register(name, {title, mount}).
 */
"use strict";

window.YM = (function () {
  const TOKEN_KEY = "ym_token";
  let token = localStorage.getItem(TOKEN_KEY) || "";
  const views = {};

  class ApiError extends Error {
    constructor(status, message) { super(message); this.status = status; }
  }

  async function api(path, opts = {}) {
    const headers = Object.assign({}, opts.headers || {});
    if (token) headers["Authorization"] = "Bearer " + token;
    let res;
    try {
      res = await fetch(path, Object.assign({}, opts, { headers }));
    } catch (_) {
      throw new ApiError(0, "cannot reach server");
    }
    if (res.status === 401) throw new ApiError(401, "authentication required");
    if (res.status === 403) throw new ApiError(403, "forbidden for this token");
    if (res.status === 404) throw new ApiError(404, "not found");
    if (!res.ok) {
      let msg = "request failed (" + res.status + ")";
      try { msg = (await res.json()).error || msg; } catch (_) {}
      throw new ApiError(res.status, msg);
    }
    const ct = res.headers.get("content-type") || "";
    return ct.includes("application/json") ? res.json() : res.text();
  }

  // ── DOM helpers ──────────────────────────────────────────────────────────
  function el(tag, cls, text) {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }
  const div = (cls, text) => el("div", cls, text);

  function preview(s, n = 80) {
    const t = (s || "").replace(/\s+/g, " ").trim();
    return t.length > n ? t.slice(0, n) + "…" : t;
  }
  function fmtTime(s) {
    if (!s) return "—";
    const d = new Date(s);
    return isNaN(d) ? s : d.toLocaleString();
  }
  function badge(conf) { return el("span", "badge " + (conf || "inferred"), conf || "inferred"); }

  // ── Connection dot + banner ───────────────────────────────────────────────
  function setConn(state) {
    const dot = document.getElementById("conn-dot");
    if (dot) dot.className = "conn-dot " + state;
  }
  function clearBanner() {
    const b = document.getElementById("banner");
    if (b) { b.hidden = true; b.textContent = ""; }
  }
  function showBanner(kind, text) {
    const b = document.getElementById("banner");
    if (!b) return;
    b.hidden = false;
    b.className = "banner " + kind;
    b.textContent = text;
  }
  function handleError(e) {
    if (e && e.status === 401) {
      setConn("warn");
      showBanner("warn", "Authentication required — enter a bearer token above and click Connect. (A loopback server with no tokens needs none.)");
    } else if (e && e.status === 403) {
      setConn("ok");
      showBanner("warn", "This token doesn't have access to that. " + (e.message || ""));
    } else if (e && e.status === 0) {
      setConn("err");
      showBanner("err", "Cannot reach the server. Is `yourmemory serve` still running?");
    } else {
      setConn("err");
      showBanner("err", "Error: " + (e && e.message ? e.message : e));
    }
  }

  // ── Health bar ("is it alive") ────────────────────────────────────────────
  function fmtBytes(n) {
    if (!n) return "0 B";
    const u = ["B", "KB", "MB", "GB"]; let i = 0; let v = n;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return v.toFixed(i ? 1 : 0) + " " + u[i];
  }
  function setHealth(state, text) {
    const bar = document.getElementById("health-bar");
    const txt = document.getElementById("health-text");
    if (bar) bar.className = "health-bar health-bar--" + state;
    if (txt) txt.textContent = text;
  }
  async function refreshHealth() {
    try {
      const h = await api("/api/health");
      const parts = [
        h.drawer_count + (h.drawer_count === 1 ? " memory" : " memories"),
        h.room_count + (h.room_count === 1 ? " room" : " rooms"),
        "last write " + fmtTime(h.last_write),
        fmtBytes(h.db_size_bytes),
      ];
      if (h.version) parts.push("v" + h.version);
      setHealth("ok", "live · " + parts.join(" · "));
    } catch (e) {
      if (e && e.status === 0) setHealth("err", "offline — is `yourmemory serve` running?");
      else if (e && e.status === 401) setHealth("unknown", "connect a token to see status");
      else setHealth("err", "status unavailable: " + (e && e.message ? e.message : e));
    }
  }

  // ── Router ────────────────────────────────────────────────────────────────
  function register(name, view) { views[name] = view; }
  function currentName() { return (location.hash || "#rooms").replace(/^#/, "") || "rooms"; }

  async function route() {
    const name = currentName();
    const view = views[name] || views.rooms;
    document.querySelectorAll(".nav-item").forEach((n) =>
      n.classList.toggle("nav-item--active", n.dataset.view === name));
    const title = document.getElementById("page-title");
    if (title) title.textContent = view.title;
    const container = document.getElementById("view");
    container.innerHTML = "";
    clearBanner();
    refreshHealth();
    try {
      setConn("ok");
      await view.mount(container);
    } catch (e) {
      handleError(e);
    }
  }

  function start() {
    const input = document.getElementById("token-input");
    const save = document.getElementById("token-save");
    const refresh = document.getElementById("refresh");
    input.value = token;
    save.addEventListener("click", () => {
      token = input.value.trim();
      if (token) localStorage.setItem(TOKEN_KEY, token);
      else localStorage.removeItem(TOKEN_KEY);
      route();
    });
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") save.click(); });
    refresh.addEventListener("click", route);
    document.querySelectorAll(".nav-item").forEach((n) =>
      n.addEventListener("click", () => { if (n.dataset.view) location.hash = "#" + n.dataset.view; }));
    window.addEventListener("hashchange", route);
    route();
  }

  return {
    api, ApiError, register, start,
    el, div, preview, fmtTime, badge,
    setConn, showBanner, clearBanner, handleError,
    getToken: () => token,
  };
})();

document.addEventListener("DOMContentLoaded", () => window.YM.start());
