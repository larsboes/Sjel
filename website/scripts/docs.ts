// website/scripts/docs.ts — the user docs: every Markdown file under website/content/, rendered
// with a section sidebar and an on-this-page column.
//
//   bun website/scripts/docs.ts --out <site>     write <site>/docs/**
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, posix } from "node:path";
import { CONTENT, renderMarkdown, type Rendered } from "./markdown.ts";
import { esc, page, REPO } from "./style.ts";

const ROOT = join(import.meta.dir, "../..");

/** Sidebar sections, in reading order. A folder not named here is listed after them. */
const SECTIONS: Array<[string, string]> = [
  ["", "Start here"],
  ["architecture", "Architecture"],
  ["capabilities", "Capabilities"],
  ["operations", "Operations"],
];

interface Doc { rel: string; from: string; page: string; rendered: Rendered }

export function collect(root = ROOT): Doc[] {
  const dir = join(root, CONTENT);
  return readdirSync(dir, { recursive: true })
    .map(String)
    .filter((f) => f.endsWith(".md"))
    .map((rel) => {
      const from = `${CONTENT}/${rel}`;
      const page = `docs/${rel.replace(/\.md$/, ".html")}`;
      return { rel, from, page, rendered: renderMarkdown(readFileSync(join(dir, rel), "utf8"), from, page) };
    });
}

function section(rel: string): string {
  return rel.includes("/") ? rel.slice(0, rel.indexOf("/")) : "";
}

export function sidebar(docs: Doc[], current: string): string {
  const known = new Set(SECTIONS.map(([key]) => key));
  const extra = [...new Set(docs.map((d) => section(d.rel)))].filter((s) => !known.has(s)).sort();
  const groups = [...SECTIONS, ...extra.map((s) => [s, s] as [string, string])];
  return `<nav aria-label="Documentation">${groups
    .map(([key, label]) => {
      const list = docs
        .filter((d) => section(d.rel) === key)
        .sort((a, b) => Number(!a.rel.endsWith("index.md")) - Number(!b.rel.endsWith("index.md")) || a.rendered.title.localeCompare(b.rendered.title));
      if (list.length === 0) return "";
      return `<h4>${esc(label)}</h4><ul>${list
        .map((d) => {
          const href = posix.relative(posix.dirname(current), d.page);
          return `<li><a href="${href}"${d.page === current ? ' aria-current="page"' : ""}>${esc(d.rendered.title)}</a></li>`;
        })
        .join("")}</ul>`;
    })
    .join("")}</nav>`;
}

function toc(r: Rendered): string | undefined {
  if (r.headings.length < 2) return undefined;
  return `<nav aria-label="On this page"><h4>On this page</h4><ul>${r.headings
    .map((h) => `<li><a${h.level === 3 ? ' class="sub"' : ""} href="#${h.id}">${esc(h.text)}</a></li>`)
    .join("")}</ul></nav>`;
}

export function build(out: string, root = ROOT): number {
  const docs = collect(root);
  for (const doc of docs) {
    const depth = doc.page.split("/").length - 1;
    const html = page({
      title: `${doc.rendered.title} — Sjel docs`,
      description: `${doc.rendered.title}: Sjel documentation.`,
      root: "../".repeat(depth),
      current: "docs",
      sidebar: sidebar(docs, doc.page),
      toc: toc(doc.rendered),
      body: doc.rendered.html,
      footer: `<p><a href="${REPO}/edit/main/${esc(doc.from)}">Edit this page</a> · rendered from <code>${esc(doc.from)}</code></p>`,
    });
    const file = join(out, doc.page);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, html);
  }
  return docs.length;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const outIdx = args.indexOf("--out");
  const out = outIdx >= 0 ? args[outIdx + 1] : join(ROOT, "site");
  console.log(`docs: wrote ${build(out)} pages`);
}
