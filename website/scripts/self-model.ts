#!/usr/bin/env bun
// website/scripts/self-model.ts — self.json rendered as one page: every unit, the coupling scan
// and every upstream verdict. Generated, never hand-maintained: a fact that is not in a manifest
// cannot appear here.
//
//   bun website/scripts/self-model.ts --out <site>     write <site>/reference/self-model.html
//   bun website/scripts/self-model.ts --check          render to memory, write nothing

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

import { esc, page } from "./style.ts";

const SJEL_ROOT = resolve(dirname(new URL(import.meta.url).pathname), "../..");

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
  capability: "A service Sjel runs, declared by one service.toml the shared runner interprets.",
  spine: "The parts every capability resolves against rather than reimplements.",
  lib: "Shared code with no service of its own.",
  pack: "Harness-neutral skills, deployed to an agent harness rather than run.",
};
// Preference order from upstreams.toml's own header comment, so the page groups them the way the
// manifest ranks them rather than alphabetically.
const VERDICT_ORDER = ["adopt", "contribute", "overlay", "fork", "build", "inspiration", "quarry", "reject", "watch"];

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
  <p class="blurb">Every external project Sjel touches, with the verdict it was given. No entry, no entry. Each one tracks its upstream\u2019s latest release; the register records no version.</p>
${body}
</section>`;
}

export function renderSite(model: SelfModel, root = "../"): string {
  const counts = KIND_ORDER
    .map((k) => [k, model.units.filter((u) => u.kind === k).length] as const)
    .filter(([, n]) => n > 0);
  return page({
    title: "Sjel — one tree that knows what it runs",
    description:
      "What Sjel is: its capabilities, their contracts, and how they connect — generated from the repository's own manifests.",
    root,
    current: "reference",
    body: `
<header>
  <h1>Sjel</h1>
  <p class="tagline">One tree that knows what it runs. Every capability declares what it needs in a manifest; one shared runner decides how to satisfy it on this machine.</p>
  <ul class="totals">
${counts.map(([k, n]) => `    <li><strong>${n}</strong> ${esc(KIND_LABEL[k].toLowerCase())}</li>`).join("\n")}
    <li><strong>${model.upstreams.length}</strong> upstreams</li>
    <li><strong>${model.coupling.length}</strong> couplings</li>
  </ul>
</header>

${renderUnits(model.units)}
${renderCoupling(model.coupling)}
${renderUpstreams(model.upstreams)}`,
    footer: `  <p>Generated from <code>self.json</code> (schema ${esc(model.schema)}, written by <code>${esc(model.generator)}</code>) and rendered by <code>website/scripts/self-model.ts</code>. Nothing here is hand-maintained.</p>`,
  });
}

function main(): void {
  const args = process.argv.slice(2);
  const outIdx = args.indexOf("--out");
  const outDir = join(outIdx >= 0 ? args[outIdx + 1] : join(SJEL_ROOT, "site"), "reference");
  const selfPath = join(SJEL_ROOT, "self.json");
  if (!existsSync(selfPath)) {
    console.error(`self-model: no self.json at ${selfPath}; run tools/self generate`);
    process.exit(1);
  }
  const model = JSON.parse(readFileSync(selfPath, "utf8")) as SelfModel;
  if (model.schema !== 2) {
    console.error(`self-model: self.json is schema ${model.schema}, this renderer knows schema 2`);
    process.exit(1);
  }
  const html = renderSite(model);
  if (args.includes("--check")) {
    console.log(`self-model: renders ${html.length} bytes (nothing written)`);
    return;
  }
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, "self-model.html"), html);
  console.log(`self-model: wrote ${join(outDir, "self-model.html")}`);
}

if (import.meta.main) main();
