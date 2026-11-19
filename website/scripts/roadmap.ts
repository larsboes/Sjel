import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, posix } from "node:path";
import { isOpen, loadWork, outcomeState, type Work, type Outcome } from "../../tools/lib/work.ts";
import { renderMarkdown } from "./markdown.ts";
import { esc, page } from "./style.ts";

const ROOT = join(import.meta.dir, "../..");
const docLink = (path: string, from: string) => posix.relative(posix.dirname(from), path.replace(/\.md$/, ".html"));

function card(outcome: Outcome, from: string): string {
  return `<a class="roadmap-card" href="${esc(docLink(outcome.path, from))}"><span class="roadmap-card-title">${esc(outcome.title)}</span><p class="roadmap-card-desc">${esc(outcome.summary)}</p></a>`;
}
function section(title: string, content: string): string {
  return `<section class="roadmap-section"><div class="roadmap-status"><h2>${esc(title)}</h2></div><div class="roadmap-cards">${content}</div></section>`;
}

/** Render only current outcomes; accepted work and publication history stay distinct. */
export function renderRoadmap(work = loadWork(ROOT)): string {
  const ordered = [...work.outcomes].sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
  const groups = [["active", "Active"], ["planned", "Planned"], ["awaiting-release", "Awaiting release"]] as const;
  const sections = groups.map(([state, title]) => {
    const outcomes = ordered.filter(o => outcomeState(work, o) === state);
    return outcomes.length ? section(title, outcomes.map(o => card(o, "roadmap/index.html")).join("")) : "";
  }).join("");
  return `<div class="roadmap-wrap"><h1>Roadmap</h1><p>What is being built and what comes next. Plans may change.</p><p><a href="delivered/index.html">Delivered</a> · <a href="../releases/index.html">Releases</a> · <a href="../docs/tasks/index.html">Open tasks</a></p>${sections || "<p>No upcoming outcomes are selected.</p>"}</div>`;
}

function delivered(work: Work): string {
  const groups = [...work.receipts].sort((a, b) => b.published_at.localeCompare(a.published_at)).map(receipt => {
    const release = work.releases.find(r => r.version === receipt.version)!;
    const outcomes = work.outcomes.filter(o => outcomeState(work, o) === "delivered" && o.tasks.some(id => release.tasks.includes(id))
      && !work.receipts.some(later => later.published_at > receipt.published_at && work.releases.find(r => r.version === later.version)!.tasks.some(id => o.tasks.includes(id))));
    return section(receipt.version, `<p><time datetime="${esc(receipt.published_at)}">${esc(receipt.published_at.slice(0, 10))}</time> · <a href="../../releases/${receipt.version}/index.html">Release notes</a></p>${outcomes.map(o => card(o, "roadmap/delivered/index.html")).join("")}`);
  }).join("");
  return `<div class="roadmap-wrap"><h1>Delivered</h1><p>Outcomes backed by published releases.</p><p><a href="../index.html">Upcoming work</a></p>${groups || "<p>No published releases have verified delivery records yet.</p>"}</div>`;
}

/** Build static views from the same records used by work checks and release preparation. */
export function build(out: string, root = ROOT): void {
  const work = loadWork(root);
  const write = (path: string, title: string, body: string) => {
    const prefix = "../".repeat(path.split("/").length - 1);
    const html = page({ title: `${title} — Sjel`, description: title, root: prefix, current: "roadmap", body,
      footer: `<p>Built from repository Markdown · <a href="${prefix}roadmap/index.html">Roadmap</a></p>` });
    const file = join(out, path); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, html);
  };
  write("roadmap/index.html", "Roadmap", renderRoadmap(work));
  write("roadmap/delivered/index.html", "Delivered", delivered(work));
  const releases = [...work.releases].sort((a, b) => {
    const av = a.version.slice(1).split(".").map(Number), bv = b.version.slice(1).split(".").map(Number);
    return bv[0] - av[0] || bv[1] - av[1] || bv[2] - av[2];
  });
  write("releases/index.html", "Releases", `<h1>Releases</h1>${releases.length ? `<ul>${releases.map(r => `<li><a href="${r.version}/index.html">${esc(r.title)}</a> — ${work.receipts.some(p => p.version === r.version) ? "Published" : "Draft; not available"}</li>`).join("")}</ul>` : "<p>No release records yet. A source tag or draft alone does not establish delivery.</p>"}`);
  for (const release of releases) {
    const path = `releases/${release.version}/index.html`;
    const receipt = work.receipts.find(p => p.version === release.version);
    const availability = receipt ? `<p>Published ${esc(receipt.published_at.slice(0, 10))} · <a href="${esc(receipt.url)}">Download release</a></p>` : "<p>Draft. Not published or available for installation.</p>";
    const tasks = release.tasks.map(id => work.tasks.find(t => t.id === id)!);
    write(path, release.title, `${availability}${renderMarkdown(release.body, release.path, path).html}<h2>${receipt ? "Delivered work" : "Intended scope"}</h2><ul>${tasks.map(t => `<li><a href="${docLink(t.path, path)}">${esc(t.title)}</a>${!receipt && t.state !== "done" ? " — acceptance pending" : ""}</li>`).join("")}</ul>`);
  }
  write("docs/tasks/index.html", "Open tasks", `<h1>Open tasks</h1><p>Actionable work. Completed and cancelled tasks are excluded.</p><ul>${work.tasks.filter(isOpen).sort((a, b) => a.priority - b.priority || a.title.localeCompare(b.title)).map(t => `<li><a href="${posix.basename(t.path, ".md")}.html">${esc(t.title)}</a> — P${t.priority}, ${esc(t.state)}${t.blocked_by ? `; blocked: ${esc(t.blocked_by)}` : ""}</li>`).join("")}</ul>`);
}

if (import.meta.main) {
  const args = process.argv.slice(2), index = args.indexOf("--out");
  build(index < 0 ? join(ROOT, "site") : args[index + 1]);
  console.log("roadmap: built upcoming, delivered, release and task views");
}
