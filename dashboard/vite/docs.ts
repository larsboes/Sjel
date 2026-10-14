import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import type { Plugin } from "vite";

const ID = "virtual:sjel-docs";
const RESOLVED = `\0${ID}`;

export interface DocFile {
  slug: string;
  category: string;
  file: string;
  markdown: string;
}

function scanDocs(dir: string, baseDir: string, plugin: { addWatchFile: (id: string) => void }): DocFile[] {
  if (!existsSync(dir)) return [];
  plugin.addWatchFile(dir);
  const entries = readdirSync(dir, { withFileTypes: true });
  const docs: DocFile[] = [];

  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      docs.push(...scanDocs(full, baseDir, plugin));
    } else if (entry.isFile() && entry.name.endsWith(".md")) {
      plugin.addWatchFile(full);
      const rel = relative(baseDir, full);
      // Derive slug: "README.md" -> "index", "architecture/index.md" -> "architecture", "architecture/storage.md" -> "architecture/storage"
      let slug = rel.replace(/\.md$/, "");
      if (slug === "README") slug = "index";
      else if (slug.endsWith("/index")) slug = slug.slice(0, -"/index".length);

      const parts = slug.split("/");
      const category = parts.length > 1 ? parts[0] : "general";

      docs.push({
        slug,
        category,
        file: `docs/${rel}`,
        markdown: readFileSync(full, "utf8"),
      });
    }
  }

  return docs;
}

/**
 * docs/*.md bundled into the /docs routes.
 *
 * Read at build time: docs are tracked source, reproducible on every machine and the published demo.
 * Only the /docs routes import the module, so the text lands in their lazy chunk.
 */
export function docs(root: string): Plugin {
  return {
    name: "sjel-docs",
    resolveId: (id) => (id === ID ? RESOLVED : undefined),
    load(id) {
      if (id !== RESOLVED) return undefined;
      const docsDir = join(root, "docs");
      const list = scanDocs(docsDir, docsDir, this);
      list.sort((a, b) => a.slug.localeCompare(b.slug));
      return `export const entries = ${JSON.stringify(list)};\n`;
    },
  };
}
