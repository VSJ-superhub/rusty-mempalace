/* Room Navigator view: Wing → Room → Drawer tree with a detail panel. */
"use strict";
(function () {
  const { api, div, preview, fmtTime, badge } = YM;
  const PAGE_SIZE = 50;

  function mount(container) {
    const split = div("split");
    const treePane = div("tree-pane");
    const tree = div("tree");
    tree.id = "rooms-tree";
    treePane.appendChild(tree);

    const detailPane = div("detail-pane");
    const detail = div("detail");
    detail.id = "rooms-detail";
    detail.appendChild(emptyDetail());
    detailPane.appendChild(detail);

    split.append(treePane, detailPane);
    container.appendChild(split);

    return loadWings(tree, detail);
  }

  function emptyDetail() {
    const wrap = div("detail-empty");
    wrap.appendChild(div("detail-empty-icon", "▤"));
    wrap.appendChild(div(null, "Select a drawer to inspect its contents, confidence, source, and validity."));
    return wrap;
  }

  async function loadWings(tree, detail) {
    tree.innerHTML = "";
    tree.appendChild(div("tree-loading", "Loading palace…"));
    const wings = await api("/api/wings");
    tree.innerHTML = "";
    if (!wings.length) {
      tree.appendChild(div("tree-empty", "No wings visible. Add a fact with `yourmemory persist`, or check your token's grants."));
      return;
    }
    for (const w of wings) tree.appendChild(wingNode(w, detail));
  }

  function wingNode(wing, detail) {
    const node = div("node");
    const row = div("node-row");
    const twisty = div("twisty", "▶");
    const label = div("node-label", wing.name);
    row.append(twisty, div("node-icon wing", "⬡"), label);
    node.appendChild(row);
    const children = div("children");
    children.style.display = "none";
    node.appendChild(children);

    let loaded = false;
    row.addEventListener("click", async () => {
      const open = children.style.display !== "none";
      if (open) { children.style.display = "none"; twisty.classList.remove("open"); return; }
      children.style.display = ""; twisty.classList.add("open");
      if (!loaded) {
        children.appendChild(div("tree-loading", "Loading rooms…"));
        const rooms = await api(`/api/wings/${wing.id}/rooms`);
        children.innerHTML = "";
        if (!rooms.length) children.appendChild(div("tree-empty", "No rooms."));
        for (const r of rooms) children.appendChild(roomNode(r, detail));
        loaded = true;
      }
    });
    return node;
  }

  function roomNode(room, detail) {
    const node = div("node");
    const row = div("node-row");
    const twisty = div("twisty", "▶");
    row.append(twisty, div("node-icon room", "○"), div("node-label", room.name));
    node.appendChild(row);
    const children = div("children");
    children.style.display = "none";
    node.appendChild(children);

    let offset = 0, loaded = false;
    async function loadPage() {
      const old = children.querySelector(".load-more");
      if (old) old.remove();
      const loading = div("tree-loading", "Loading drawers…");
      children.appendChild(loading);
      const page = await api(`/api/rooms/${room.id}/drawers?limit=${PAGE_SIZE}&offset=${offset}`);
      loading.remove();
      if (offset === 0 && !page.items.length) { children.appendChild(div("tree-empty", "No drawers.")); return; }
      for (const d of page.items) children.appendChild(drawerRow(d, detail));
      offset += page.items.length;
      if (page.count === page.limit) {
        const more = div("load-more", "Load more…");
        more.addEventListener("click", (ev) => { ev.stopPropagation(); loadPage(); });
        children.appendChild(more);
      }
    }
    row.addEventListener("click", () => {
      const open = children.style.display !== "none";
      if (open) { children.style.display = "none"; twisty.classList.remove("open"); return; }
      children.style.display = ""; twisty.classList.add("open");
      if (!loaded) { loaded = true; loadPage(); }
    });
    return node;
  }

  function drawerRow(d, detail) {
    const row = div("drawer-row" + (d.is_invalidated ? " invalid" : ""));
    row.append(badge(d.confidence), div("drawer-preview", preview(d.content)));
    row.addEventListener("click", (ev) => {
      ev.stopPropagation();
      document.querySelectorAll(".drawer-row.selected").forEach((n) => n.classList.remove("selected"));
      row.classList.add("selected");
      showDetail(detail, d.id);
    });
    return row;
  }

  let activeId = null;
  async function showDetail(detail, id) {
    activeId = id;
    detail.innerHTML = "";
    detail.appendChild(div("tree-loading", "Loading drawer…"));
    const d = await api(`/api/drawers/${id}`);
    if (activeId !== id) return;
    detail.innerHTML = "";

    const head = div("detail-head");
    head.append(div("detail-title", "Drawer"), div("detail-id", "#" + d.id), badge(d.confidence));
    if (d.is_invalidated) head.appendChild(div("invalid-flag", "invalidated"));
    detail.appendChild(head);

    detail.appendChild(Object.assign(div("detail-content"), { textContent: d.content }));

    const dl = document.createElement("dl");
    dl.className = "meta-grid";
    const rows = [
      ["Source", d.source], ["Confidence", d.confidence],
      ["Access count", String(d.access_count)],
      ["Created", fmtTime(d.created_at)], ["Last accessed", fmtTime(d.last_accessed_at)],
      ["Wing / Room", `${d.wing_id} / ${d.room_id}`],
    ];
    if (d.is_invalidated) rows.push(["Invalidated at", fmtTime(d.invalidated_at)]);
    for (const [k, v] of rows) {
      const dt = document.createElement("dt"); dt.textContent = k;
      const dd = document.createElement("dd"); dd.textContent = v;
      dl.append(dt, dd);
    }
    detail.appendChild(dl);
  }

  YM.register("rooms", { title: "Room Navigator", mount });
})();
