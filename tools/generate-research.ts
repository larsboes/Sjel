#!/usr/bin/env bun
// Renders research/*.md into the demo site's /research pages (ISA.md, ISC-15). The markdown
// files are the source; these pages are a build artifact, like /docs.
//
//   bun tools/generate-research.ts --out <dir>     write <dir>/index.html and <dir>/<entry>.html

import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { SJEL_ROOT } from "./lib/demo-endpoints.ts";
import { page } from "./lib/site-style.ts";

const REPO = "https://github.com/larsboes/Sjel/blob/main";

/** A link between entries stays on the site; a link to any other repository file goes to GitHub. */
export function rewriteLinks(html: string): string {
  return html.replace(/href="([^"]+)"/g, (whole, href: string) => {
    if (/^[a-z]+:/i.test(href) || href.startsWith("#")) return whole;
    const [path, anchor] = href.split("#");
    if (!path.includes("/") && path.endsWith(".md")) {
      const name = path === "README.md" ? "index" : path.slice(0, -3);
      return `href="${name}.html${anchor ? `#${anchor}` : ""}"`;
    }
    const repoPath = join("research", path).replace(/^research\/\.\.\//, "").replace(/^\.\.\//, "");
    return `href="${REPO}/${repoPath}${anchor ? `#${anchor}` : ""}"`;
  });
}

function main(): void {
  const args = process.argv.slice(2);
  const outIdx = args.indexOf("--out");
  const outDir = outIdx >= 0 ? args[outIdx + 1] : join(SJEL_ROOT, "site/research");
  const srcDir = join(SJEL_ROOT, "research");
  mkdirSync(outDir, { recursive: true });
  const files = readdirSync(srcDir).filter((f) => f.endsWith(".md"));
  for (const file of files) {
    const md = readFileSync(join(srcDir, file), "utf8");
    const title = md.match(/^# (.+)$/m)?.[1] ?? file;
    const name = file === "README.md" ? "index" : file.slice(0, -3);
    writeFileSync(
      join(outDir, `${name}.html`),
      page({
        title: `${title} — Sjel research`,
        description: `Sjel research: ${title}.`,
        root: "../",
        current: "research",
        body: rewriteLinks(Bun.markdown.html(md)),
        footer: `<p>Rendered from <a href="${REPO}/research/${file}"><code>research/${file}</code></a>.</p>`,
      }),
    );
  }
  console.log(`wrote ${files.length} pages to ${outDir}`);
}

if (import.meta.main) main();
