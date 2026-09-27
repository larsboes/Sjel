#!/usr/bin/env bun
// tools/generate-site.ts — render self.json as a self-contained page (#14).
//
// The repository has no landing surface: the only entry points are a 32 KB README and a generated
// ARCHITECTURE.md. This renders what Axon already knows about itself into one HTML file someone
// can read without cloning.
//
// Generated, never hand-maintained, for the same reason ARCHITECTURE.md is: a fact that is not in
// a manifest cannot appear here, so the page cannot drift from the tree.
//
// WHAT IS DELIBERATELY NOT RENDERED, and this is the interesting part:
//
//   graph.*        node counts and the stale list come from graphify-out/, which is untracked.
//                  A fresh clone cannot reproduce them, so publishing them would put a number on
//                  the page that nobody can check — the exact property #7 fixed for ports.
//   units[].code   same reason: per-unit file and node counts come from that graph.
//
// Everything that remains traces to a tracked manifest: service.toml for kind/port/requires,
// upstreams.toml for the verdict, and tools/self.ts's own coupling scan over Cargo.toml and
// source imports.
//
// Self-contained by requirement (#14) and by house rule: inline CSS, no script, no font, no image,
// no request to any other host. The one interactive element (filtering by kind) is CSS-only.
//
// Since #168 this is the site's LANDING page rather than the whole site: the reference pages
// (tools/generate-docs) and the live demo (tools/demo-site) sit beside it, and the shell, the
// stylesheet and the nav that ties the three together moved to tools/lib/site-style.ts. What
// stayed here is unchanged — the self-model tables, and the argument above about which of
// them may honestly be published.
//
//   tools/generate-site                 write site/index.html
//   tools/generate-site --out <dir>     write <dir>/index.html
//   tools/generate-site --check         render to memory, report size, write nothing

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

import { loadManifest } from "./lib/demo-endpoints.ts";
import { esc, page } from "./lib/site-style.ts";

const SJEL_ROOT = resolve(dirname(new URL(import.meta.url).pathname), "..");

interface Unit {
  name: string;
  kind: "capability" | "lib" | "pack" | "spine";
  service?: { kind: string; requires: string[]; port?: string; image?: string };
}
interface Coupling { from: string; to: string; kinds: string[]; evidence: string[] }
interface Upstream { name: string; verdict: string }
interface SelfModel { schema: number; generator: string; units: Unit[]; coupling: Coupling[]; upstreams: Upstream[] }


const KIND_ORDER: Unit["kind"][] = ["capability", "spine", "lib", "pack"];
const KIND_LABEL: Record<string, string> = {
  capability: "Capabilities",
  spine: "Spine",
  lib: "Libraries",
  pack: "Packs",
};
const KIND_BLURB: Record<string, string> = {
  capability: "A service Axon runs, declared by one service.toml the shared runner interprets.",
  spine: "The parts every capability resolves against rather than reimplements.",
  lib: "Shared code with no service of its own.",
  pack: "Harness-neutral skills, deployed to an agent harness rather than run.",
};
// Preference order from upstreams.toml's own header comment, so the page groups them the way the
// manifest ranks them rather than alphabetically.
const VERDICT_ORDER = ["adopt", "contribute", "overlay", "fork", "build", "inspiration", "quarry", "reject"];

function renderUnits(units: Unit[]): string {
  return KIND_ORDER.map((kind) => {
    const rows = units.filter((u) => u.kind === kind).sort((a, b) => a.name.localeCompare(b.name));
    if (rows.length === 0) return "";
    const body = rows.map((u) => {
      const svc = u.service;
      const runs = svc ? esc(svc.kind) : "—";
      const port = svc?.port ? `<code>${esc(svc.port)}</code>` : "—";
      const needs = svc?.requires?.length
        ? svc.requires.map((r) => `<code>${esc(r)}</code>`).join(" ")
        : "—";
      return `<tr><th scope="row">${esc(u.name)}</th><td>${runs}</td><td>${port}</td><td>${needs}</td></tr>`;
    }).join("\n");
    return `
<section id="${kind}">
  <h2>${KIND_LABEL[kind]} <span class="count">${rows.length}</span></h2>
  <p class="blurb">${KIND_BLURB[kind]}</p>
  <div class="scroll">
    <table>
      <thead><tr><th scope="col">Name</th><th scope="col">Runs as</th><th scope="col">Port</th><th scope="col">Requires</th></tr></thead>
      <tbody>
${body}
      </tbody>
    </table>
  </div>
</section>`;
  }).join("\n");
}

function renderCoupling(coupling: Coupling[]): string {
  if (coupling.length === 0) return "";
  const rows = [...coupling]
    .sort((a, b) => a.from.localeCompare(b.from) || a.to.localeCompare(b.to))
    .map((c) => `<tr><th scope="row">${esc(c.from)}</th><td>→ ${esc(c.to)}</td><td>${c.kinds.map((k) => `<code>${esc(k)}</code>`).join(" ")}</td></tr>`)
    .join("\n");
  return `
<section id="coupling">
  <h2>Coupling <span class="count">${coupling.length}</span></h2>
  <p class="blurb">Which unit reads which, found in Cargo path dependencies and source imports rather than declared by hand.</p>
  <div class="scroll">
    <table>
      <thead><tr><th scope="col">From</th><th scope="col">To</th><th scope="col">Seen in</th></tr></thead>
      <tbody>
${rows}
      </tbody>
    </table>
  </div>
</section>`;
}

function renderUpstreams(upstreams: Upstream[]): string {
  const groups = VERDICT_ORDER
    .map((v) => [v, upstreams.filter((u) => u.verdict === v).sort((a, b) => a.name.localeCompare(b.name))] as const)
    .filter(([, list]) => list.length > 0);
  const body = groups.map(([verdict, list]) => `
    <h3>${esc(verdict)} <span class="count">${list.length}</span></h3>
    <ul class="upstream-list">
${list.map((u) => `      <li><span>${esc(u.name)}</span></li>`).join("\n")}
    </ul>`).join("\n");
  return `
<section id="upstreams">
  <h2>Upstreams <span class="count">${upstreams.length}</span></h2>
  <p class="blurb">Every external project Axon touches, with the verdict it was given. No entry, no entry. Each one tracks its upstream\u2019s latest release; the register records no version.</p>
${body}
</section>`;
}

export function renderSite(model: SelfModel, demoed: string[] = []): string {
  const counts = KIND_ORDER
    .map((k) => [k, model.units.filter((u) => u.kind === k).length] as const)
    .filter(([, n]) => n > 0);
  return page({
    title: "Axon — one tree that knows what it runs",
    description:
      "What Axon is: its capabilities, their contracts, and how they connect — generated from the repository's own manifests, with a live demo running on synthetic data.",
    // Lives at /docs/self-model.html since #170, so one level up.
    root: "../",
    current: "overview",
    body: `
<header>
  <h1>Axon</h1>
  <p class="tagline">One tree that knows what it runs. Every capability declares what it needs in a manifest; one shared runner decides how to satisfy it on this machine.</p>
  <ul class="totals">
${counts.map(([k, n]) => `    <li><strong>${n}</strong> ${esc(KIND_LABEL[k].toLowerCase())}</li>`).join("\n")}
    <li><strong>${model.upstreams.length}</strong> upstreams</li>
    <li><strong>${model.coupling.length}</strong> couplings</li>
  </ul>
</header>

<section>
  <h2>Where else to look</h2>
  <p class="blurb">Nothing on this site is written by hand. The tables below are generated from the manifests, the reference pages from those plus a live recording, and the dashboard at the root runs against data that was never real.</p>
  <ul class="cards">
    <li><a href="../"><strong>Dashboard →</strong><span>The actual shell, on ${demoed.length} capabilities seeded with generated data. Writing is disabled; nothing in it is real.</span></a></li>
    <li><a href="index.html"><strong>Reference →</strong><span>One page per unit: declared contract, what it is coupled to, and response shapes read out of the recording.</span></a></li>
    <li><a href="https://github.com/larsboes/Sjel"><strong>Source →</strong><span>The repository, its README, and the manifests every page here is generated from.</span></a></li>
  </ul>
</section>
${renderUnits(model.units)}
${renderCoupling(model.coupling)}
${renderUpstreams(model.upstreams)}`,
    footer: `  <p>Generated from <code>self.json</code> (schema ${esc(model.schema)}, written by <code>${esc(model.generator)}</code>) and rendered by <code>tools/generate-site.ts</code>. Nothing here is hand-maintained.</p>
  <p>Code-graph counts and the per-unit file counts in <code>self.json</code> are deliberately absent: they come from an untracked artifact, so a fresh clone cannot reproduce them, and a number nobody can check does not belong on a page like this.</p>`,
  });
}

function main(): void {
  const args = process.argv.slice(2);
  if (args.includes("-h") || args.includes("--help")) {
    console.log("tools/generate-site [--out <dir>] [--name <file>] [--check]");
    process.exit(0);
  }
  const outIdx = args.indexOf("--out");
  const outDir = outIdx >= 0 ? args[outIdx + 1] : join(SJEL_ROOT, "site");
  // Since #170 the site root is the dashboard, so this page is no longer an index. It keeps
  // its own generator because it renders self.json rather than a per-unit view, and it is
  // written into /docs beside the reference pages under its own name.
  const nameIdx = args.indexOf("--name");
  const outName = nameIdx >= 0 ? args[nameIdx + 1] : "index.html";
  const selfPath = join(SJEL_ROOT, "self.json");
  if (!existsSync(selfPath)) {
    console.error(`generate-site: no self.json at ${selfPath} — run: tools/self generate`);
    process.exit(1);
  }
  const model = JSON.parse(readFileSync(selfPath, "utf8")) as SelfModel;
  // A schema bump means the shape this renderer reads may have moved under it. Fail rather than
  // publish a page built from assumptions about fields that no longer mean what they did.
  if (model.schema !== 1) {
    console.error(`generate-site: self.json is schema ${model.schema}, this renderer knows schema 1`);
    process.exit(1);
  }
  // Which capabilities the demo actually runs, read from demo.toml rather than inferred from
  // what is NOT in its absent list — the demo covers a handful of the tree's capabilities, and
  // "everything except the three we wrote a reason for" would overstate it by twenty.
  const html = renderSite(model, loadManifest().capabilities.map((c) => c.name));
  if (args.includes("--check")) {
    console.log(`generate-site: renders ${html.length} bytes from schema ${model.schema} (nothing written)`);
    return;
  }
  mkdirSync(outDir, { recursive: true });
  const dest = join(outDir, outName);
  writeFileSync(dest, html);
  console.log(`wrote ${dest} (${html.length} bytes)`);
}

if (import.meta.main) main();
