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
  currentSlug: string,
  link: (path: string) => string,
): string {
  if (/^[a-z]+:/i.test(href) || href.startsWith("#")) return href;

  const [rawPath, anchor] = href.split("#");
  const hash = anchor ? `#${anchor}` : "";

  if (!rawPath) return hash;

  // If pointing to a markdown file
  if (rawPath.endsWith(".md")) {
    const clean = rawPath.replace(/\.md$/, "");
    let target = clean;

    if (clean.startsWith("/")) {
      target = clean.slice(1);
    } else if (clean.startsWith("../")) {
      const parts = currentSlug.split("/").slice(0, -1);
      const segs = clean.split("/");
      for (const seg of segs) {
        if (seg === "..") parts.pop();
        else if (seg !== ".") parts.push(seg);
      }
      target = parts.join("/");
    } else if (clean.startsWith("./")) {
      const currentDir = currentSlug.includes("/") ? currentSlug.slice(0, currentSlug.lastIndexOf("/")) : "";
      target = currentDir ? `${currentDir}/${clean.slice(2)}` : clean.slice(2);
    } else {
      const currentDir = currentSlug.includes("/") ? currentSlug.slice(0, currentSlug.lastIndexOf("/")) : "";
      target = currentDir ? `${currentDir}/${clean}` : clean;
    }

    if (target.endsWith("/index")) target = target.slice(0, -"/index".length);
    if (target === "index" || target === "README") return link(`/docs${hash}`);
    return link(`/docs/${target}${hash}`);
  }

  // Any other repository path goes to GitHub
  const repoPath = rawPath.startsWith("/") ? rawPath.slice(1) : rawPath;
  return `${REPO}/${repoPath}${hash}`;
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
