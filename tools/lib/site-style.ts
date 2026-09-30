// tools/lib/site-style.ts — the published site's one stylesheet (#168).
//
// Extracted from tools/generate-site.ts when the site grew a second and third page. Three
// generators emitting three near-identical <style> blocks is three places for the landing
// page and the reference pages to drift apart, and the drift is always visible to exactly
// the person you least want to show it to.
//
// Inlined into every page rather than served as a file: a self-contained page is a house
// rule here, and at this size a shared .css file would buy one fewer round trip at the cost
// of the property that any one of these pages still renders correctly when saved to disk.

export const SITE_CSS = `
  :root {
    color-scheme: light dark;
    --bg: #fbfbfa; --fg: #1a1a1a; --dim: #5b5b58; --line: #e2e1dd;
    --card: #ffffff; --accent: #2b5f8f; --code-bg: #f2f1ee; --warn: #8a5a1c;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --bg: #121312; --fg: #e8e7e3; --dim: #9a9a95; --line: #2a2b29;
      --card: #191a19; --accent: #7fb3dd; --code-bg: #202120; --warn: #d8a860;
    }
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 0 1.25rem 5rem; background: var(--bg); color: var(--fg);
    font: 15px/1.6 ui-sans-serif, -apple-system, "Segoe UI", Roboto, sans-serif;
  }
  main { max-width: 62rem; margin: 0 auto; }
  header { padding: 3rem 0 2rem; border-bottom: 1px solid var(--line); }
  h1 { margin: 0 0 .4rem; font-size: 2.1rem; letter-spacing: -.02em; }
  .tagline { margin: 0; max-width: 44rem; color: var(--dim); font-size: 1.05rem; }
  .totals { display: flex; flex-wrap: wrap; gap: .5rem; margin: 1.6rem 0 0; padding: 0; list-style: none; }
  .totals li {
    padding: .35rem .7rem; border: 1px solid var(--line); border-radius: 999px;
    background: var(--card); font-size: .82rem; color: var(--dim);
  }
  .totals strong { color: var(--fg); font-variant-numeric: tabular-nums; }
  section { padding: 2.5rem 0 0; }
  h2 { margin: 0 0 .3rem; font-size: 1.3rem; letter-spacing: -.01em; }
  h3 { margin: 1.6rem 0 .5rem; font-size: .95rem; text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
  .count {
    margin-left: .4rem; padding: .1rem .45rem; border-radius: 4px; background: var(--code-bg);
    font-size: .75rem; font-weight: 400; color: var(--dim); vertical-align: middle;
    font-variant-numeric: tabular-nums;
  }
  .blurb { margin: 0 0 1rem; max-width: 46rem; color: var(--dim); font-size: .9rem; }
  /* Wide content scrolls inside its own box; the page body never scrolls sideways. */
  .scroll { overflow-x: auto; border: 1px solid var(--line); border-radius: 8px; background: var(--card); }
  table { width: 100%; border-collapse: collapse; font-size: .88rem; }
  th, td { padding: .55rem .8rem; text-align: left; border-bottom: 1px solid var(--line); vertical-align: top; }
  tbody tr:last-child th, tbody tr:last-child td { border-bottom: 0; }
  thead th { font-size: .72rem; text-transform: uppercase; letter-spacing: .06em; color: var(--dim); font-weight: 500; }
  tbody th { font-weight: 600; white-space: nowrap; }
  code {
    padding: .1rem .35rem; border-radius: 4px; background: var(--code-bg);
    font: .85em ui-monospace, SFMono-Regular, Menlo, monospace; overflow-wrap: anywhere;
  }
  .upstream-list { margin: 0; padding: 0; list-style: none; display: grid; gap: .3rem;
          grid-template-columns: repeat(auto-fill, minmax(19rem, 1fr)); }
  .upstream-list li {
    display: flex; align-items: baseline; justify-content: space-between; gap: .6rem;
    padding: .4rem .7rem; border: 1px solid var(--line); border-radius: 6px; background: var(--card);
    font-size: .85rem;
  }
  footer { margin-top: 4rem; padding-top: 1.5rem; border-top: 1px solid var(--line); color: var(--dim); font-size: .82rem; }
  footer code { background: none; padding: 0; }
  a { color: var(--accent); }

  /* Site navigation, present on every page so none of them is a dead end. */
  .sitenav {
    display: flex; flex-wrap: wrap; gap: .4rem; align-items: baseline;
    padding: .9rem 0 0; font-size: .85rem;
  }
  .sitenav a { padding: .25rem .6rem; border: 1px solid var(--line); border-radius: 999px; text-decoration: none; }
  .sitenav a[aria-current] { border-color: var(--accent); }
  .sitenav .sep { flex: 1 }

  /* Top brand header with hedgehog mark */
  .site-header {
    display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between;
    gap: .75rem; padding: 1.25rem 0 .75rem; border-bottom: 1px solid var(--line); margin-bottom: 1.5rem;
  }
  .site-brand {
    display: inline-flex; align-items: center; gap: .6rem; text-decoration: none;
    color: var(--fg); font-weight: 600; font-size: 1.15rem; letter-spacing: -.01em;
  }
  .site-brand:hover { color: var(--accent); }
  .site-logo {
    display: block; width: 28px; height: 28px; border-radius: 6px; flex-shrink: 0;
  }
  .site-header .sitenav { padding: 0; }

  /* Cards: the landing page's three ways in, and the docs index. */
  .cards { display: grid; gap: .75rem; grid-template-columns: repeat(auto-fill, minmax(17rem, 1fr)); padding: 0; margin: 1rem 0 0; list-style: none; }
  .cards li { border: 1px solid var(--line); border-radius: 8px; background: var(--card); }
  .cards a { display: block; padding: .9rem 1rem; text-decoration: none; color: inherit; height: 100% }
  .cards a:hover { border-color: var(--accent); }
  .cards strong { display: block; font-size: .95rem; }
  .cards span { display: block; margin-top: .25rem; color: var(--dim); font-size: .82rem; }

  /* A stated limitation, never styled as an error: an honest gap is information. */
  .note {
    margin: 1rem 0 0; padding: .8rem 1rem; border-left: 3px solid var(--warn);
    background: var(--card); color: var(--dim); font-size: .86rem;
  }
  .note strong { color: var(--warn); }
`;

/** Minimal, total escaping. Every value on this site comes from a tracked manifest or a
 *  recorded response, and both are still text somebody edits. */
export const esc = (s: unknown): string =>
  String(s ?? "")
    .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;").replace(/'/g, "&#39;");

export interface PageOptions {
  title: string;
  description: string;
  /** Prefix back to the site root: "" at the root, "../" one level down. */
  root: string;
  current: "overview" | "docs" | "demo" | "research";
  body: string;
  footer: string;
}

/** One page shell, so the nav and the head cannot differ between generators. */
export function page(opts: PageOptions): string {
  const link = (href: string, label: string, key: PageOptions["current"]) =>
    `<a href="${opts.root}${href}"${opts.current === key ? ' aria-current="page"' : ""}>${label}</a>`;
  // The root is the running dashboard, not a generated page (#170), so "Dashboard" is an
  // empty href against `root` rather than a filename.
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(opts.title)}</title>
<meta name="description" content="${esc(opts.description)}">
<link rel="icon" href="${opts.root}favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" sizes="180x180" href="${opts.root}apple-touch-icon.png">
<style>${SITE_CSS}</style>
</head>
<body>
<main>
<header class="site-header">
  <a class="site-brand" href="${opts.root}">
    <img class="site-logo" src="${opts.root}favicon.svg" alt="" width="28" height="28">
    <span>Sjel</span>
  </a>
  <nav class="sitenav" aria-label="Site">
    ${link("", "Dashboard", "demo")}
    ${link("docs/index.html", "Reference", "docs")}
    ${link("docs/self-model.html", "Self-model", "overview")}
    ${link("research/index.html", "Research", "research")}
    <span class="sep"></span>
    <a href="https://github.com/larsboes/Sjel">Source</a>
  </nav>
</header>
${opts.body}
<footer>
${opts.footer}
<p class="sitenav">
  ${link("", "Dashboard", "demo")}
  ${link("docs/index.html", "Reference", "docs")}
  ${link("docs/self-model.html", "Self-model", "overview")}
  ${link("research/index.html", "Research", "research")}
  <span class="sep"></span>
  <a href="https://github.com/larsboes/Sjel">Source</a>
</p>
</footer>
</main>
</body>
</html>
`;
}
