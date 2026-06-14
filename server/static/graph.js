/* Graph Explorer view: D3 force-directed knowledge graph.
 * Wings = hexagon, Rooms = circle, Drawers = square. Relation edges are accent-colored;
 * expired relations render dashed/greyed. Drawer nodes are tinted by confidence.
 */
"use strict";
(function () {
  const { api, div } = YM;

  const KIND_COLOR = { wing: "#388bfd", room: "#8b949e", drawer: "#3fb950" };
  const CONF_COLOR = { high: "#3fb950", medium: "#d29922", low: "#f85149", inferred: "#f85149" };

  async function mount(container) {
    if (!window.d3) {
      container.appendChild(div("pane-msg", "D3 failed to load (offline?). The graph needs https://d3js.org/d3.v7.min.js — see frontend.md for vendoring it locally."));
      return;
    }
    const wrap = div("graph-wrap");
    const legend = div("graph-legend");
    legend.innerHTML =
      '<span><i class="lg hex"></i>Wing</span><span><i class="lg cir"></i>Room</span>' +
      '<span><i class="lg sq"></i>Drawer</span><span><i class="lg edge"></i>relation</span>' +
      '<span><i class="lg edge dash"></i>expired</span>';
    const info = div("graph-info");
    info.id = "graph-info";
    info.appendChild(div("pane-msg", "Click a node to inspect it. Scroll to zoom, drag to pan."));
    wrap.append(legend, info);
    container.appendChild(wrap);

    const data = await api("/api/graph");
    if (data.truncated) {
      YM.showBanner("warn", `Graph truncated to ${data.nodes.filter(n => n.kind === "drawer").length} drawers for performance.`);
    }
    if (!data.nodes.length) {
      wrap.appendChild(div("pane-msg", "No graph data visible for this token."));
      return;
    }
    render(wrap, data, info);
  }

  function render(wrap, data, info) {
    const W = wrap.clientWidth || 900;
    const H = wrap.clientHeight || 600;
    const nodes = data.nodes.map((n) => Object.assign({}, n));
    const links = data.edges.map((e) => Object.assign({}, e));

    const svg = d3.select(wrap).append("svg")
      .attr("width", W).attr("height", H).attr("class", "graph-svg");
    const root = svg.append("g");

    svg.call(d3.zoom().scaleExtent([0.2, 4]).on("zoom", (ev) => root.attr("transform", ev.transform)));

    const link = root.append("g").selectAll("line")
      .data(links).join("line")
      .attr("class", (d) => "edge" + (d.kind === "relation" ? " rel" : " struct") + (d.expired ? " expired" : ""));
    link.append("title").text((d) => d.label + (d.expired ? " (expired)" : ""));

    const node = root.append("g").selectAll("g")
      .data(nodes).join("g")
      .attr("class", "gnode")
      .call(d3.drag()
        .on("start", (ev, d) => { if (!ev.active) sim.alphaTarget(0.3).restart(); d.fx = d.x; d.fy = d.y; })
        .on("drag", (ev, d) => { d.fx = ev.x; d.fy = ev.y; })
        .on("end", (ev, d) => { if (!ev.active) sim.alphaTarget(0); d.fx = null; d.fy = null; }));

    node.each(function (d) {
      const g = d3.select(this);
      const fill = d.kind === "drawer" ? (CONF_COLOR[d.confidence] || KIND_COLOR.drawer) : KIND_COLOR[d.kind];
      if (d.kind === "wing") g.append("path").attr("d", hexagon(11)).attr("fill", fill);
      else if (d.kind === "room") g.append("circle").attr("r", 8).attr("fill", fill);
      else g.append("rect").attr("x", -7).attr("y", -7).attr("width", 14).attr("height", 14).attr("rx", 2).attr("fill", fill);
    });
    node.append("text").attr("class", "gnode-label").attr("x", 14).attr("dy", "0.32em")
      .text((d) => d.label.length > 30 ? d.label.slice(0, 30) + "…" : d.label);
    node.append("title").text((d) => `${d.kind}: ${d.label}`);
    node.on("click", (ev, d) => { ev.stopPropagation(); selectNode(info, d); });

    const sim = d3.forceSimulation(nodes)
      .force("link", d3.forceLink(links).id((d) => d.id).distance((d) => d.kind === "relation" ? 90 : 50).strength(0.5))
      .force("charge", d3.forceManyBody().strength(-180))
      .force("center", d3.forceCenter(W / 2, H / 2))
      .force("collide", d3.forceCollide(18))
      .on("tick", () => {
        link.attr("x1", (d) => d.source.x).attr("y1", (d) => d.source.y)
            .attr("x2", (d) => d.target.x).attr("y2", (d) => d.target.y);
        node.attr("transform", (d) => `translate(${d.x},${d.y})`);
      });
  }

  function hexagon(r) {
    const pts = [];
    for (let i = 0; i < 6; i++) {
      const a = (Math.PI / 3) * i - Math.PI / 2;
      pts.push([r * Math.cos(a), r * Math.sin(a)]);
    }
    return "M" + pts.map((p) => p.join(",")).join("L") + "Z";
  }

  async function selectNode(info, d) {
    info.innerHTML = "";
    const head = div("graph-info-head");
    head.append(div("detail-title", d.kind), div("detail-id", d.label));
    info.appendChild(head);
    if (d.kind !== "drawer") {
      info.appendChild(div("pane-msg", d.wing ? "Wing: " + d.wing : ""));
      return;
    }
    const id = d.id.split(":")[1];
    info.appendChild(div("tree-loading", "Loading drawer…"));
    try {
      const dr = await api(`/api/drawers/${id}`);
      info.innerHTML = "";
      const h = div("graph-info-head");
      h.append(div("detail-title", "Drawer"), div("detail-id", "#" + dr.id), YM.badge(dr.confidence));
      info.appendChild(h);
      info.appendChild(Object.assign(div("detail-content"), { textContent: dr.content }));
      info.appendChild(div("meta-line", `${dr.source} · accessed ${dr.access_count}× · ${YM.fmtTime(dr.created_at)}`));
    } catch (e) { YM.handleError(e); }
  }

  YM.register("graph", { title: "Graph Explorer", mount });
})();
