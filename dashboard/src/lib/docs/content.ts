/**
 * docs/ data structures and helpers for the in-app documentation browser.
 */

export interface DocEntry {
  slug: string;
  category: string;
  file: string;
  title: string;
  body: string;
  summary: string;
}

export interface DocSection {
  id: string;
  label: string;
  description: string;
  docs: DocEntry[];
}

const REPO = "https://github.com/larsboes/Sjel/blob/main";

export function toDocEntry(doc: { slug: string; category: string; file: string; markdown: string }): DocEntry {
  const title = doc.markdown.match(/^# (.+)$/m)?.[1]?.trim() ?? doc.slug;
  const body = doc.markdown.replace(/^# .+\n+/m, "");
  // First paragraph as quick summary
  const summaryMatch = body.match(/^([^#\n\r`|>-][^\n\r]+)/m);
  const summary = summaryMatch ? summaryMatch[1].trim() : "";

  return {
    slug: doc.slug,
    category: doc.category,
    file: doc.file,
    title,
    body,
    summary,
  };
}

/**
 * Resolves a relative link in a document to an in-app /docs route or GitHub repo link.
 */
export function resolveDocLink(
  href: string,
  currentFile: string,
  link: (path: string) => string,
): string {
  if (/^[a-z]+:/i.test(href) || href.startsWith("#") || href.startsWith("//")) return href;

  // The source file retains the directory that section-index route slugs omit.
  const resolved = new URL(href, `https://sjel.invalid/${currentFile}`);
  const path = resolved.pathname.slice(1);
  const suffix = resolved.search + resolved.hash;
  if (path.startsWith("docs/") && path.endsWith(".md")) {
    let target = path.slice("docs/".length, -".md".length);
    if (target.endsWith("/index")) target = target.slice(0, -"/index".length);
    if (target === "index" || target === "README") return link(`/docs/README${suffix}`);
    return link(`/docs/${target}${suffix}`);
  }
  return `${REPO}/${path}${suffix}`;
}

export const CATEGORY_META: Record<string, { label: string; description: string; order: number }> = {
  general: {
    label: "Getting Started",
    description: "System overview, topology, and reading guides.",
    order: 1,
  },
  architecture: {
    label: "Architecture",
    description: "Core boundaries, single-writer SQLite, and security models.",
    order: 2,
  },
  capabilities: {
    label: "Capabilities",
    description: "Service manifests, lifecycle orchestration, and freshness contracts.",
    order: 3,
  },
  operations: {
    label: "Operations",
    description: "Host diagnostics, doctor health probes, and daemon persistence.",
    order: 4,
  },
};

export function groupDocs(docs: DocEntry[]): DocSection[] {
  const groups: Record<string, DocEntry[]> = {};

  for (const doc of docs) {
    if (!groups[doc.category]) groups[doc.category] = [];
    groups[doc.category].push(doc);
  }

  return Object.entries(groups)
    .map(([id, list]) => {
      const meta = CATEGORY_META[id] ?? {
        label: id.charAt(0).toUpperCase() + id.slice(1),
        description: "",
        order: 99,
      };
      // Sort docs in section: index first, then alphabetical by title
      const sorted = [...list].sort((a, b) => {
        if (a.slug === id || a.slug === "index") return -1;
        if (b.slug === id || b.slug === "index") return 1;
        return a.title.localeCompare(b.title);
      });
      return {
        id,
        label: meta.label,
        description: meta.description,
        order: meta.order,
        docs: sorted,
      };
    })
    .sort((a, b) => a.order - b.order);
}
