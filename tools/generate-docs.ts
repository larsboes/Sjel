#!/usr/bin/env bun
// tools/generate-docs.ts — a reference page per unit, from the manifests (#168).
//
// Same contract as ARCHITECTURE.md and the landing page: a fact that is not declared
// somewhere in this tree cannot appear here. Nothing below is hand-written prose about a
// capability — it is service.toml, self.json's coupling scan, demo.toml, and where a demo
// recording exists, the shape of what the capability actually answered.
//
// WHAT IS DELIBERATELY NOT HERE: rendered README prose. Every capability has a long README
// and rendering it would need a Markdown engine in the tools tree, which is a dependency
// bought to restate something GitHub already renders from the same file. So each page links
// to the README and spends its own space on what GitHub cannot show: the declared contract,
// what the unit is coupled to, and a real response shape.
//
// THE ENDPOINT TABLES ARE THE INTERESTING PART. Where demo/fixtures holds a recording, the
// page derives each response's field names and types from the bytes a real server sent. That
// is API documentation nobody wrote and nobody can forget to update — it is regenerated from
// a live capability on every build, and a field that disappears from the server disappears
// from the page in the same commit.
//
//   tools/generate-docs --out <dir>     write <dir>/index.html and <dir>/<unit>.html

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { SJEL_ROOT, loadManifest } from "./lib/demo-endpoints.ts";
import { esc, page } from "./lib/site-style.ts";

interface Unit {
  name: string;
  kind: "capability" | "lib" | "pack" | "spine";
  service?: { kind: string; requires: string[]; port?: string; image?: string };
}
interface Coupling { from: string; to: string; kinds: string[]; evidence: string[] }
interface SelfModel { schema: number; generator: string; units: Unit[]; coupling: Coupling[] }

const KIND_LABEL: Record<string, string> = {
  capability: "Capability",
  spine: "Spine",
  lib: "Library",
  pack: "Pack",
};

const REPO = "https://github.com/larsboes/Sjel/blob/main";

/** Where a unit's own source lives, by kind. The one piece of layout knowledge here, and it
 *  is the repository's own directory convention rather than a per-unit fact. */
function sourceDir(unit: Unit): string {
  switch (unit.kind) {
    case "capability": return `capabilities/${unit.name}`;
    case "lib": return `libs/${unit.name}`;
    case "pack": return `Packs/${unit.name}`;
    default: return unit.name;
  }
}

// ─── Response shapes ──────────────────────────────────────────────────────────

/** A one-word type for a JSON value, as a reader of an API would name it. */
function typeOf(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return value.length > 0 ? `array of ${typeOf(value[0])}` : "array";
  switch (typeof value) {
    case "number": return Number.isInteger(value) ? "integer" : "number";
    case "boolean": return "boolean";
    case "string": return /^\d{4}-\d{2}-\d{2}/.test(value) ? "date-string" : "string";
    case "object": return "object";
    default: return typeof value;
  }
}

/**
 * Field names and types one level into a recorded response.
 *
 * One level on purpose. A finance dashboard nests six deep and a full expansion would be
 * longer than the source that produced it, at which point nobody reads either. The top level
 * plus a type per field is what tells a reader whether the endpoint returns what they need —
 * and the fixture itself is published beside this page for anyone who wants the rest.
 */
function fields(body: unknown): Array<{ name: string; type: string }> {
  const sample = Array.isArray(body) ? body[0] : body;
  if (!sample || typeof sample !== "object") return [];
  return Object.entries(sample as Record<string, unknown>)
    .map(([name, value]) => ({ name, type: typeOf(value) }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

interface Recording { path: string; rows: number | null; fields: Array<{ name: string; type: string }> }

/** Read what the recorder captured for one capability. Absent fixtures are normal: a docs
 *  build can legitimately run before, or without, a demo recording. */
function recordings(capability: string, fixturesDir: string): Recording[] {
  const indexPath = join(fixturesDir, "index.json");
  if (!existsSync(indexPath)) return [];
  const index = JSON.parse(readFileSync(indexPath, "utf8")) as {
    prefixes: Array<{ capability: string; prefix: string }>;
    routes: Record<string, string>;
  };
  const owned = index.prefixes.filter((p) => p.capability === capability).map((p) => p.prefix);
  const out: Recording[] = [];
  for (const [path, file] of Object.entries(index.routes)) {
    if (!owned.some((p) => path === p || path.startsWith(`${p}/`) || path.startsWith(`${p}?`))) continue;
    const full = join(fixturesDir, file);
    if (!existsSync(full)) continue;
    const body = JSON.parse(readFileSync(full, "utf8"));
    out.push({ path, rows: Array.isArray(body) ? body.length : null, fields: fields(body) });
  }
  return out.sort((a, b) => a.path.localeCompare(b.path));
}

// ─── Rendering ────────────────────────────────────────────────────────────────

function contractTable(unit: Unit): string {
  const svc = unit.service;
  const rows: Array<[string, string]> = [["Kind", KIND_LABEL[unit.kind] ?? unit.kind]];
  if (svc) {
    rows.push(["Runs as", esc(svc.kind)]);
    if (svc.port) rows.push(["Port", `<code>${esc(svc.port)}</code>`]);
    if (svc.image) rows.push(["Image", `<code>${esc(svc.image)}</code>`]);
    rows.push([
      "Requires",
      svc.requires?.length ? svc.requires.map((r) => `<code>${esc(r)}</code>`).join(" ") : "nothing",
    ]);
  } else {
    rows.push(["Service", "none — this unit is code, not something that runs"]);
  }
  return `<div class="scroll"><table><tbody>
${rows.map(([k, v]) => `<tr><th scope="row">${esc(k)}</th><td>${v}</td></tr>`).join("\n")}
</tbody></table></div>`;
}

function couplingSection(unit: Unit, coupling: Coupling[]): string {
  const out = coupling.filter((c) => c.from === unit.name);
  const inbound = coupling.filter((c) => c.to === unit.name);
  if (out.length === 0 && inbound.length === 0) return "";
  const list = (rows: Coupling[], other: (c: Coupling) => string) =>
    `<ul class="pins">${rows
      .map((c) => `<li><span>${esc(other(c))}</span><code>${c.kinds.map(esc).join(" ")}</code></li>`)
      .join("")}</ul>`;
  return `
<section>
  <h2>Coupling</h2>
  <p class="blurb">Found in Cargo path dependencies and source imports, not declared by hand. Compile-time, unlike <code>Requires</code> above, which is what has to be running.</p>
  ${out.length ? `<h3>Reads <span class="count">${out.length}</span></h3>${list(out, (c) => c.to)}` : ""}
  ${inbound.length ? `<h3>Read by <span class="count">${inbound.length}</span></h3>${list(inbound, (c) => c.from)}` : ""}
</section>`;
}

function endpointSection(recs: Recording[], absentReason: string | undefined): string {
  if (absentReason) {
    return `
<section>
  <h2>Endpoints</h2>
  <p class="note"><strong>Not in the demo recording.</strong> ${esc(absentReason)}</p>
</section>`;
  }
  if (recs.length === 0) return "";
  return `
<section>
  <h2>Endpoints <span class="count">${recs.length}</span></h2>
  <p class="blurb">Derived from what this capability actually answered while the demo was recorded — field names and types are read out of the response, never written down here.</p>
  ${recs
    .map(
      (r) => `
  <h3>${esc(r.path)}${r.rows !== null ? ` <span class="count">${r.rows} rows</span>` : ""}</h3>
  ${
    r.fields.length === 0
      ? `<p class="blurb">Answered with no object to describe${r.rows === 0 ? " — the recording is an empty list" : ""}.</p>`
      : `<div class="scroll"><table>
    <thead><tr><th scope="col">Field</th><th scope="col">Type</th></tr></thead>
    <tbody>${r.fields.map((f) => `<tr><th scope="row">${esc(f.name)}</th><td><code>${esc(f.type)}</code></td></tr>`).join("")}</tbody>
  </table></div>`
  }`,
    )
    .join("\n")}
</section>`;
}

function unitPage(unit: Unit, model: SelfModel, recs: Recording[], absentReason?: string): string {
  const dir = sourceDir(unit);
  return page({
    title: `${unit.name} — Axon reference`,
    description: `The declared contract, coupling and recorded response shapes for Axon's ${unit.name} ${(KIND_LABEL[unit.kind] ?? unit.kind).toLowerCase()}.`,
    root: "../",
    current: "docs",
    body: `
<header>
  <h1>${esc(unit.name)}</h1>
  <p class="tagline">${esc(KIND_LABEL[unit.kind] ?? unit.kind)} · <a href="${REPO}/${esc(dir)}">source</a> · <a href="${REPO}/${esc(dir)}/README.md">README</a></p>
</header>
<section>
  <h2>Contract</h2>
  <p class="blurb">Every field below is read from <code>${esc(dir)}/service.toml</code>, the one file that declares how this unit runs.</p>
  ${contractTable(unit)}
</section>
${couplingSection(unit, model.coupling)}
${endpointSection(recs, absentReason)}`,
    footer: `<p>Generated by <code>tools/generate-docs.ts</code> from <code>self.json</code> (schema ${esc(model.schema)}) and the demo recording. Nothing on this page is hand-maintained.</p>`,
  });
}

function indexPage(units: Unit[], model: SelfModel, absent: Record<string, string>): string {
  const groups = (["capability", "spine", "lib", "pack"] as const)
    .map((kind) => [kind, units.filter((u) => u.kind === kind)] as const)
    .filter(([, list]) => list.length > 0);
  return page({
    title: "Axon — reference",
    description: "One reference page per Axon capability, library and Pack, generated from the repository's own manifests.",
    root: "../",
    current: "docs",
    body: `
<header>
  <h1>Reference</h1>
  <p class="tagline">One page per unit, generated from <code>service.toml</code>, the coupling scan in <code>self.json</code>, and the response shapes recorded from the live demo.</p>
</header>
${groups
  .map(
    ([kind, list]) => `
<section>
  <h2>${esc(KIND_LABEL[kind])}${list.length === 1 ? "" : kind === "capability" ? " (capabilities)" : "s"} <span class="count">${list.length}</span></h2>
  <ul class="cards">
${list
  .sort((a, b) => a.name.localeCompare(b.name))
  .map((u) => {
    const svc = u.service;
    const detail = absent[u.name]
      ? "Not in the live demo"
      : svc
        ? `${svc.kind}${svc.port ? ` · port ${svc.port}` : ""}${svc.requires?.length ? ` · needs ${svc.requires.join(", ")}` : ""}`
        : "no service of its own";
    return `    <li><a href="${esc(u.name)}.html"><strong>${esc(u.name)}</strong><span>${esc(detail)}</span></a></li>`;
  })
  .join("\n")}
  </ul>
</section>`,
  )
  .join("\n")}`,
    footer: `<p>Generated by <code>tools/generate-docs.ts</code>. ${model.units.length} units, ${model.coupling.length} couplings.</p>`,
  });
}

function main(): void {
  const args = process.argv.slice(2);
  if (args.includes("-h") || args.includes("--help")) {
    console.log("tools/generate-docs [--out <dir>] [--fixtures <dir>]");
    return;
  }
  const outIdx = args.indexOf("--out");
  const outDir = outIdx >= 0 ? args[outIdx + 1] : join(SJEL_ROOT, "site/docs");
  const manifest = loadManifest();
  const fixIdx = args.indexOf("--fixtures");
  const fixturesDir = fixIdx >= 0 ? args[fixIdx + 1] : join(SJEL_ROOT, manifest.fixturesDir);

  const model = JSON.parse(readFileSync(join(SJEL_ROOT, "self.json"), "utf8")) as SelfModel;
  // A schema bump means the shape read below may have moved. Fail rather than publish pages
  // built from fields that no longer mean what they did.
  //
  // Schema 2 (2026-09-29) dropped the per-unit `code` counts and the `graph` block. This file
  // never read either — the fields it does read (`units`, `coupling`, and the `service` block
  // inside a unit) are unchanged — so only the accepted version moves. tools/generate-site.ts
  // took the same bump; this consumer was missed, and the Pages build was red for it from
  // 2026-09-29 17:25 until here.
  if (model.schema !== 2) {
    console.error(`generate-docs: self.json is schema ${model.schema}, this generator knows schema 2`);
    process.exit(1);
  }

  mkdirSync(outDir, { recursive: true });
  for (const unit of model.units) {
    writeFileSync(
      join(outDir, `${unit.name}.html`),
      unitPage(unit, model, recordings(unit.name, fixturesDir), manifest.absent[unit.name]),
    );
  }
  writeFileSync(join(outDir, "index.html"), indexPage(model.units, model, manifest.absent));
  console.log(`wrote ${model.units.length + 1} pages to ${outDir}`);
}

if (import.meta.main) main();
