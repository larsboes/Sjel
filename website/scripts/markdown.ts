// website/scripts/markdown.ts — Markdown to HTML for the site, with Bun's built-in renderer.
//
// Links are rewritten so the rendered page works where it is published: a link into
// website/content/ becomes the matching page on the site, an in-repository link becomes the
// file on GitHub, and an external or in-page link is left alone.
import { posix } from "node:path";
import { REPO } from "./style.ts";

export const CONTENT = "website/content";

export interface Rendered {
  title: string;
  html: string;
  /** Second- and third-level headings, for the on-this-page column. */
  headings: Array<{ level: 2 | 3; id: string; text: string }>;
}

function stripTags(html: string): string {
  return html.replace(/<[^>]+>/g, "").replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">").replaceAll("&quot;", '"');
}

/**
 * `href` as the published page needs it. `from` is the repository path of the Markdown file,
 * `page` the site path of the page it becomes (for example `docs/architecture/index.html`).
 */
export function rewriteHref(href: string, from: string, page: string): string {
  if (/^([a-z][a-z0-9+.-]*:|#|\/\/)/i.test(href)) return href;
  const [path, frag] = href.split("#", 2) as [string, string | undefined];
  if (!path) return href;
  const target = posix.normalize(posix.join(posix.dirname(from), decodeURI(path)));
  const hash = frag ? `#${frag}` : "";
  if (target.startsWith(`${CONTENT}/`) && target.endsWith(".md")) {
    const sitePath = `docs/${target.slice(CONTENT.length + 1).replace(/\.md$/, ".html")}`;
    return posix.relative(posix.dirname(page), sitePath) + hash;
  }
  if (target === CONTENT || target === `${CONTENT}/`) {
    return posix.relative(posix.dirname(page), "docs/index.html") + hash;
  }
  const kind = target.endsWith("/") || !posix.extname(target) ? "tree" : "blob";
  return `${REPO}/${kind}/main/${target.replace(/\/$/, "")}${hash}`;
}

export function renderMarkdown(src: string, from: string, page: string): Rendered {
  let html = Bun.markdown.html(src, { headings: { ids: true } });
  html = html.replace(/<!--[\s\S]*?-->/g, "");
  html = html.replace(/(<a\s[^>]*href=")([^"]+)(")/g, (_, a, href, b) => a + rewriteHref(href, from, page) + b);
  html = html.replace(/(<img\s[^>]*src=")([^"]+)(")/g, (_, a, src, b) => {
    const url = rewriteHref(src, from, page);
    return a + (url.startsWith(`${REPO}/blob/`) ? url.replace("/blob/", "/raw/") : url) + b;
  });
  const headings: Rendered["headings"] = [];
  html = html.replace(/<h([23]) id="([^"]+)">([\s\S]*?)<\/h\1>/g, (_, level, id, inner) => {
    headings.push({ level: Number(level) as 2 | 3, id, text: stripTags(inner) });
    return `<h${level} id="${id}">${inner}<a class="anchor" href="#${id}" aria-label="Link to this section">#</a></h${level}>`;
  });
  const h1 = html.match(/<h1[^>]*>([\s\S]*?)<\/h1>/);
  return { title: h1 ? stripTags(h1[1]) : posix.basename(from, ".md"), html, headings };
}
