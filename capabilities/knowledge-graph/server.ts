#!/usr/bin/env bun
// server.ts — serves the knowledge-graph capability: a REST API over
// graphify-out/graph.json. No UI of its own.
//
// It used to ship a second SvelteKit app on this port, which drew the whole
// 6.6k-node graph in one vis-network canvas loaded from a CDN. That never
// converged into a picture and made a local-first surface depend on unpkg. The
// browser view now lives on the dashboard's self-model page, which reads the
// self-model at unit level and calls /api/graph/unit/:name to go one level
// deeper — bounded, so it renders. This process kept the half that had no
// duplicate: the data.
//
// usage: bun capabilities/knowledge-graph/server.ts
// env:   SJEL_PORT   the port to bind (exported by tools/service-runner.sh from the manifest)
//        SJEL_ROOT   repo root (defaults to cwd)
//
// API endpoints:
//   GET  /api/graph              — full graph (nodes + edges)
//   GET  /api/graph/stats        — node/edge/community counts
//   GET  /api/graph/search?q=    — search nodes by label or file type
//   GET  /api/graph/community/:id — nodes and edges in one community
//   GET  /api/graph/node/:id     — one node with its connections
//   GET  /api/graph/unit/:name   — one unit's files and the edges among them, capped
//
// Importing this file starts nothing: binding the port and every exit live in
// main(), behind `import.meta.main`. That is what lets server.test.ts call
// handleAPI against a planted graph without a port or a running service — a
// module that exits the process on import cannot be tested at all (Axon#116).

import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const axonRoot = (): string => process.env.SJEL_ROOT ?? process.cwd();

// ── Graph data ──────────────────────────────────────────────────────────────

// Keyed on path as well as mtime: the path is derived from SJEL_ROOT, so a cache keyed on
// mtime alone can hand one root's graph to another whose file happens to share a timestamp.
let graphCache: { path: string; data: any; mtime: number } | null = null;

export function loadGraph(): any {
  const graphJsonPath = join(axonRoot(), "graphify-out/graph.json");
  if (!existsSync(graphJsonPath)) return null;
  const mtime = statSync(graphJsonPath).mtimeMs;
  if (graphCache && graphCache.path === graphJsonPath && graphCache.mtime === mtime) return graphCache.data;
  const raw = readFileSync(graphJsonPath, "utf-8");
  const data = JSON.parse(raw);
  graphCache = { path: graphJsonPath, data, mtime };
  return data;
}

interface FlatNode {
  id: string;
  label: string;
  file_type: string;
  source_file: string;
  community: number;
  group: string;
}

interface FlatEdge {
  from: string;
  to: string;
  label?: string;
}

export function flattenNodes(graph: any): FlatNode[] {
  const nodes = graph.nodes ?? [];
  return nodes.map((n: any) => ({
    id: String(n.id ?? n.label ?? ""),
    label: n.label ?? n.id ?? "",
    file_type: n.file_type ?? "unknown",
    source_file: n.source_file ?? "",
    community: n.community ?? -1,
    group: n.community !== undefined ? `community-${n.community}` : "uncategorized",
  }));
}

export function flattenEdges(graph: any): FlatEdge[] {
  const links = graph.links ?? graph.edges ?? [];
  return links.map((e: any) => ({
    from: String(e.source ?? e.from ?? ""),
    to: String(e.target ?? e.to ?? ""),
    label: e.label ?? e.relationship ?? "",
  }));
}

function getCommunities(nodes: FlatNode[]): number[] {
  const set = new Set<number>();
  for (const n of nodes) {
    if (n.community >= 0) set.add(n.community);
  }
  return [...set].sort((a, b) => a - b);
}

// ── API handlers ────────────────────────────────────────────────────────────

export function handleAPI(url: URL): Response | null {
  const path = url.pathname;

  // /api/graph — full graph
  if (path === "/api/graph") {
    const graph = loadGraph();
    if (!graph) return new Response(JSON.stringify({ error: "graph not found — run tools/graphify.sh first" }), {
      status: 404,
      headers: { "content-type": "application/json" },
    });
    return new Response(JSON.stringify(graph), {
      headers: { "content-type": "application/json", "cache-control": "no-cache" },
    });
  }

  // /api/graph/stats — summary counts
  if (path === "/api/graph/stats") {
    const graph = loadGraph();
    if (!graph) return json404("graph not found — run tools/graphify.sh first");
    const nodes = flattenNodes(graph);
    const edges = flattenEdges(graph);
    const communities = getCommunities(nodes);
    return jsonOk({
      nodes: nodes.length,
      edges: edges.length,
      communities: communities.length,
      built_at: graph.built_at_commit ?? "unknown",
      corpus_files: nodes.filter(n => n.file_type === "code").length,
      doc_files: nodes.filter(n => n.file_type !== "code").length,
    });
  }

  // /api/graph/search?q=<term>
  if (path === "/api/graph/search") {
    const q = (url.searchParams.get("q") ?? "").toLowerCase().trim();
    if (!q) return json400("missing 'q' query parameter");
    const graph = loadGraph();
    if (!graph) return json404("graph not found");
    const nodes = flattenNodes(graph).filter(n =>
      n.label.toLowerCase().includes(q) ||
      n.source_file.toLowerCase().includes(q) ||
      n.file_type.toLowerCase().includes(q)
    );
    const edges = flattenEdges(graph);
    // Return matching nodes with their edges filtered to those connecting matching nodes
    const nodeIds = new Set(nodes.map(n => n.id));
    const matchingEdges = edges.filter(e => nodeIds.has(e.from) && nodeIds.has(e.to));
    return jsonOk({
      query: q,
      results: nodes.length,
      nodes,
      edges: matchingEdges,
    });
  }

  // /api/graph/community/:id
  const communityMatch = path.match(/^\/api\/graph\/community\/(\d+)$/);
  if (communityMatch) {
    const communityId = parseInt(communityMatch[1], 10);
    const graph = loadGraph();
    if (!graph) return json404("graph not found");
    const nodes = flattenNodes(graph).filter(n => n.community === communityId);
    const edges = flattenEdges(graph);
    const nodeIds = new Set(nodes.map(n => n.id));
    const communityEdges = edges.filter(e => nodeIds.has(e.from) && nodeIds.has(e.to));
    return jsonOk({
      community: communityId,
      members: nodes.length,
      nodes,
      edges: communityEdges,
    });
  }

  // /api/graph/node/:id
  const nodeMatch = path.match(/^\/api\/graph\/node\/(.+)$/);
  if (nodeMatch) {
    const nodeId = decodeURIComponent(nodeMatch[1]);
    const graph = loadGraph();
    if (!graph) return json404("graph not found");
    const nodes = flattenNodes(graph);
    const node = nodes.find(n => n.id === nodeId);
    if (!node) return json404(`node '${nodeId}' not found`);
    const edges = flattenEdges(graph);
    const connections = edges.filter(e => e.from === nodeId || e.to === nodeId).map(e => {
      const otherId = e.from === nodeId ? e.to : e.from;
      const other = nodes.find(n => n.id === otherId);
      return {
        relationship: e.label || "connected",
        node: other || { id: otherId, label: otherId },
      };
    });
    return jsonOk({ node, connections });
  }

  // /api/graph/unit/:name — one unit's files and the edges among them
  const unitMatch = path.match(/^\/api\/graph\/unit\/(.+)$/);
  if (unitMatch) {
    const unit = decodeURIComponent(unitMatch[1]);
    const graph = loadGraph();
    if (!graph) return json404("graph not found");
    const prefixes = unitPrefixes(unit);
    if (!prefixes.length) return json400(`'${unit}' is not a unit name`);

    const all = flattenNodes(graph);
    const owned = all.filter(n => prefixes.some(p => (n.source_file ?? "").startsWith(p)));
    if (!owned.length) return json404(`no graph nodes under ${prefixes.join(" or ")}`);

    // Rank by degree before capping, so a truncated answer keeps the part of the
    // unit that explains its shape rather than an arbitrary slice of it.
    const edges = flattenEdges(graph);
    const ownedIds = new Set(owned.map(n => n.id));
    const degree = new Map<string, number>();
    for (const e of edges) {
      if (ownedIds.has(e.from)) degree.set(e.from, (degree.get(e.from) ?? 0) + 1);
      if (ownedIds.has(e.to)) degree.set(e.to, (degree.get(e.to) ?? 0) + 1);
    }
    const ranked = [...owned].sort((a, b) => (degree.get(b.id) ?? 0) - (degree.get(a.id) ?? 0));
    const nodes = ranked.slice(0, UNIT_NODE_CAP);
    const keptIds = new Set(nodes.map(n => n.id));

    return jsonOk({
      unit,
      prefixes,
      // Reported, never silent: a capped answer that looked complete would read
      // as "this is the whole unit" when it is the busiest part of it.
      total: owned.length,
      returned: nodes.length,
      truncated: owned.length > nodes.length,
      cap: UNIT_NODE_CAP,
      nodes,
      edges: edges.filter(e => keptIds.has(e.from) && keptIds.has(e.to)),
    });
  }

  return null; // not an API route
}

/**
 * How many of a unit's nodes one drill-down returns.
 *
 * A force layout stops being a picture long before it stops being a data
 * structure: the full graph is 6.6k nodes and rendered as a black rectangle,
 * which is the defect this endpoint exists to make unreachable. A few hundred
 * still lays out in well under a second and still reads as a shape.
 */
const UNIT_NODE_CAP = 400;

/**
 * Repo-relative path prefixes a unit owns.
 *
 * Mirrors `unitForPath` in tools/lib/self-model.ts, which is what named these
 * units in the first place -- the two must agree or a unit on the self-model
 * page drills into nothing. Spine directories ARE the unit and have no
 * `<name>` segment; the three nouns each nest one level down
 * (CONTRIBUTING.md#three-architectural-nouns).
 */
function unitPrefixes(unit: string): string[] {
  if (unit.includes("/") || unit.includes("..")) return [];
  if (["dashboard", "tools", "schemas"].includes(unit)) return [`${unit}/`];
  return [`capabilities/${unit}/`, `libs/${unit}/`, `Packs/${unit}/`];
}

function jsonOk(data: any): Response {
  return new Response(JSON.stringify(data), {
    headers: { "content-type": "application/json", "cache-control": "no-cache" },
  });
}

function json400(msg: string): Response {
  return new Response(JSON.stringify({ error: msg }), {
    status: 400,
    headers: { "content-type": "application/json" },
  });
}

function json404(msg: string): Response {
  return new Response(JSON.stringify({ error: msg }), {
    status: 404,
    headers: { "content-type": "application/json" },
  });
}

// ── Server ──────────────────────────────────────────────────────────────────

function main(): void {
  const port = Number(process.env.SJEL_PORT ?? 4244);
  if (!Number.isInteger(port) || port <= 0) {
    console.error(`knowledge-graph: SJEL_PORT must be a port number, got ${JSON.stringify(process.env.SJEL_PORT)}`);
    process.exit(1);
  }

  // Idle detection. The dashboard's self-model page is the only browser
  // caller, and it fetches rather than renders here, so the ping that a served
  // page used to send never arrives. Every request counts as the signal
  // instead, which is what keeps an on-demand capability alive while someone
  // is actually reading its data.
  let lastSignal = Date.now();

  Bun.serve({
    hostname: "127.0.0.1",
    port,
    fetch(request) {
      const url = new URL(request.url);
      lastSignal = Date.now();

      if (url.pathname === "/__axon/idle") {
        return Response.json({
          idle_seconds: Math.floor((Date.now() - lastSignal) / 1000),
          last_signal: new Date(lastSignal).toISOString(),
        });
      }

      if (url.pathname.startsWith("/api/")) {
        return handleAPI(url) ?? json404("unknown API endpoint");
      }

      return json404("knowledge-graph serves /api/graph* only — the browser view is the dashboard's self-model page");
    },
  });

  console.log(`knowledge-graph: serving API on 127.0.0.1:${port}`);
}

if (import.meta.main) main();
