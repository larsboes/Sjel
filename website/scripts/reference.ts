// website/scripts/reference.ts — one reference page per unit, from what the tree declares.
//
// A page is the unit's contract (service.toml through self.json), its routes (the `const
// ROUTES` block in its source), what it is coupled to (self.json's scan) and its README. No
// fact on a page is written for the page.
//
//   bun website/scripts/reference.ts --out <site>     write <site>/reference/**
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { renderMarkdown } from "./markdown.ts";
import { capabilityRoutes, type DeclaredRoute } from "./routes.ts";
import { esc, GITHUB_ICON, page, REPO } from "./style.ts";

const ROOT = join(import.meta.dir, "../..");

interface Unit {
  name: string;
  kind: "capability" | "lib" | "pack" | "spine";
  service?: { kind: string; requires: string[]; port?: string; image?: string };
}
interface Coupling { from: string; to: string; kinds: string[]; evidence: string[] }
interface SelfModel { schema: number; generator: string; units: Unit[]; coupling: Coupling[] }

const KINDS: Array<[Unit["kind"], string]> = [
  ["capability", "Capabilities"],
  ["lib", "Libraries"],
  ["pack", "Packs"],
  ["spine", "Spine"],
];
const LABEL: Record<string, string> = { capability: "Capability", lib: "Library", pack: "Pack", spine: "Spine" };

/** The repository's directory convention for a unit's source. */
function sourceDir(unit: Unit): string {
  switch (unit.kind) {
    case "capability":
    case "lib":
      return ["crates", "services"].map((dir) => `${dir}/${unit.name}`).find((p) => existsSync(join(ROOT, p))) ?? `crates/${unit.name}`;
    case "pack": return `Packs/${unit.name}`;
    default: return unit.name;
  }
}

function sidebar(units: Unit[], current: string): string {
  return `<nav aria-label="Reference"><ul><li><a href="index.html"${current === "index" ? ' aria-current="page"' : ""}>Overview</a></li>
<li><a href="self-model.html"${current === "self-model" ? ' aria-current="page"' : ""}>Self-model</a></li></ul>${KINDS.map(([kind, label]) => {
    const list = units.filter((u) => u.kind === kind).sort((a, b) => a.name.localeCompare(b.name));
    if (list.length === 0) return "";
    return `<h4>${label}</h4><ul>${list
      .map((u) => `<li><a href="${esc(u.name)}.html"${u.name === current ? ' aria-current="page"' : ""}>${esc(u.name)}</a></li>`)
      .join("")}</ul>`;
  }).join("")}</nav>`;
}

function contract(unit: Unit): string {
  const svc = unit.service;
  const rows: Array<[string, string]> = [["Kind", `<span class="pill">${esc(LABEL[unit.kind] ?? unit.kind)}</span>`]];
  if (svc) {
    const pillClass = svc.kind === "process" ? "proc" : svc.kind === "container" ? "cont" : "data";
    rows.push(["Runs as", `<span class="pill ${pillClass}">${esc(svc.kind)}</span>`]);
    if (svc.port) rows.push(["Port", `<code>${esc(svc.port)}</code>`]);
    if (svc.image) rows.push(["Image", `<code>${esc(svc.image)}</code>`]);
    rows.push(["Requires", svc.requires?.length ? svc.requires.map((r) => `<a href="${esc(r)}.html"><code>${esc(r)}</code></a>`).join(" ") : "nothing"]);
  } else {
    rows.push(["Service", "none: this unit is code, not something that runs"]);
  }
  return `<div class="scroll"><table><tbody>${rows.map(([k, v]) => `<tr><th scope="row">${esc(k)}</th><td>${v}</td></tr>`).join("")}</tbody></table></div>`;
}

function routesSection(routes: DeclaredRoute[], dir: string): string {
  if (routes.length === 0) return "";
  return `<h2 id="routes">Routes <span class="count">${routes.length}</span></h2>
<p class="blurb">Read from the <code>const ROUTES</code> block in <a href="${REPO}/tree/main/${esc(dir)}/src"><code>${esc(dir)}/src</code></a>, the same manifest the capability serves at <code>GET /routes</code>. <span class="schema">schema</span> marks a route that declares its request body.</p>
<div class="scroll"><table><thead><tr><th scope="col">Route</th><th scope="col">What it does</th></tr></thead><tbody>${routes
    .map((r) => `<tr><td class="route"><span class="method ${esc(r.method).toLowerCase()}">${esc(r.method)}</span><code>${esc(r.path)}</code>${r.schema ? ' <span class="schema">schema</span>' : ""}</td><td>${esc(r.summary).replace(/`([^`]+)`/g, "<code>$1</code>")}</td></tr>`)
    .join("")}</tbody></table></div>`;
}

function couplingSection(unit: Unit, coupling: Coupling[]): string {
  const out = coupling.filter((c) => c.from === unit.name);
  const inbound = coupling.filter((c) => c.to === unit.name);
  if (out.length === 0 && inbound.length === 0) return "";
  const list = (rows: Coupling[], other: (c: Coupling) => string) =>
    `<ul class="pins">${rows.map((c) => `<li><a href="${esc(other(c))}.html">${esc(other(c))}</a><code>${c.kinds.map(esc).join(" ")}</code></li>`).join("")}</ul>`;
  return `<h2 id="coupling">Coupling</h2>
<p class="blurb">Found in Cargo path dependencies and source imports. Compile-time, unlike <em>Requires</em>, which is what has to be running.</p>
${out.length ? `<h3>Reads <span class="count">${out.length}</span></h3>${list(out, (c) => c.to)}` : ""}
${inbound.length ? `<h3>Read by <span class="count">${inbound.length}</span></h3>${list(inbound, (c) => c.from)}` : ""}`;
}

function readme(unit: Unit, root: string): string {
  const from = `${sourceDir(unit)}/README.md`;
  const file = join(root, from);
  if (!existsSync(file)) return "";
  const rendered = renderMarkdown(readFileSync(file, "utf8"), from, `reference/${unit.name}.html`);
  // The README's own title repeats the page heading.
  const body = rendered.html.replace(/^\s*<h1[^>]*>[\s\S]*?<\/h1>/, "");
  return `<h2 id="readme">README</h2>${body}`;
}

function unitPage(unit: Unit, model: SelfModel, units: Unit[], root: string): string {
  const dir = sourceDir(unit);
  return page({
    title: `${unit.name} — Sjel reference`,
    description: `The declared contract, routes and coupling of Sjel's ${unit.name} ${(LABEL[unit.kind] ?? unit.kind).toLowerCase()}.`,
    root: "../",
    current: "reference",
    sidebar: sidebar(units, unit.name),
    body: `<h1>${esc(unit.name)}</h1>
<p class="tagline">${esc(LABEL[unit.kind] ?? unit.kind)} · <a href="${REPO}/tree/main/${esc(dir)}" class="icon-link source-link" title="Source">${GITHUB_ICON} <span>Source</span></a></p>
<h2 id="contract">Contract</h2>
${contract(unit)}
${unit.kind === "capability" ? routesSection(capabilityRoutes(root, unit.name), dir) : ""}
${couplingSection(unit, model.coupling)}
${readme(unit, root)}`,
    footer: `<p>Generated by <code>website/scripts/reference.ts</code> from <code>self.json</code> and <code>${esc(dir)}</code>.</p>`,
  });
}

function indexPage(model: SelfModel): string {
  return page({
    title: "Sjel — reference",
    description: "One reference page per Sjel capability, library and Pack, generated from the repository.",
    root: "../",
    current: "reference",
    sidebar: sidebar(model.units, "index"),
    body: `<h1>Reference</h1>
<p class="tagline">One page per unit: its contract, its routes, what it is coupled to, and its README. Generated from the tree on every build.</p>
${KINDS.map(([kind, label]) => {
  const list = model.units.filter((u) => u.kind === kind).sort((a, b) => a.name.localeCompare(b.name));
  if (list.length === 0) return "";
  return `<h2 id="${kind}">${label} <span class="count">${list.length}</span></h2><ul class="cards">${list
    .map((u) => {
      const svc = u.service;
      const pillClass = svc?.kind === "process" ? "proc" : svc?.kind === "container" ? "cont" : svc?.kind ? "data" : "";
      const kindBadge = svc ? `<span class="pill ${pillClass}">${esc(svc.kind)}</span>` : '<span class="pill">code</span>';
      const detail = svc ? `${kindBadge}${svc.port ? ` · <code>${svc.port}</code>` : ""}${svc.requires?.length ? ` · needs ${svc.requires.join(", ")}` : ""}` : kindBadge;
      return `<li><a href="${esc(u.name)}.html"><strong>${esc(u.name)}</strong><span>${detail}</span></a></li>`;
    })
    .join("")}</ul>`;
}).join("\n")}`,
    footer: `<p>${model.units.length} units, ${model.coupling.length} couplings. Generated by <code>website/scripts/reference.ts</code>.</p>`,
  });
}

export function build(out: string, root = ROOT): number {
  const model = JSON.parse(readFileSync(join(root, "self.json"), "utf8")) as SelfModel;
  if (model.schema !== 2) throw new Error(`self.json is schema ${model.schema}; reference.ts reads schema 2`);
  const dir = join(out, "reference");
  mkdirSync(dir, { recursive: true });
  for (const unit of model.units) writeFileSync(join(dir, `${unit.name}.html`), unitPage(unit, model, model.units, root));
  writeFileSync(join(dir, "index.html"), indexPage(model));
  return model.units.length + 1;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const outIdx = args.indexOf("--out");
  console.log(`reference: wrote ${build(outIdx >= 0 ? args[outIdx + 1] : join(ROOT, "site"))} pages`);
}
