// website/scripts/style.ts — the shell every page shares: tokens, the top bar, and the layouts
// for docs (sidebar, article, on-this-page) and reference pages. Inlined into each page, so a
// page saved to disk still renders.

export const REPO = "https://github.com/larsboes/Sjel";

export function esc(value: unknown): string {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

export const TOKENS = `
:root {
  color-scheme: light dark;
  --ink: #0b1d22; --ink-2: #10292f; --teal: #16748d; --teal-2: #1f8fab; --cream: #f7f4ec;
  --bg: #f3f5f5; --fg: #13201f; --dim: #55615f; --line: #d9dfde; --card: #ffffff; --code: #e9eeed;
  --link: #11627a;
  --pill-bg: #e2e8e7; --pill-fg: #2c4240;
  --method-get: #16748d; --method-post: #2a8a5b; --method-put: #b27318; --method-del: #b73e34;
  --sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, sans-serif;
  --mono: ui-monospace, "SF Mono", SFMono-Regular, Menlo, monospace;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #0e1517; --fg: #e4eae9; --dim: #93a19f; --line: #22302f; --card: #131d1f; --code: #18262a; --link: #6cc3d8;
    --pill-bg: #1c2b2d; --pill-fg: #b2c5c3;
    --method-get: #5cc2dc; --method-post: #48bb78; --method-put: #ecc94b; --method-del: #fc8181;
  }
}
:root[data-theme="dark"] {
  --bg: #0e1517; --fg: #e4eae9; --dim: #93a19f; --line: #22302f; --card: #131d1f; --code: #18262a; --link: #6cc3d8;
  --pill-bg: #1c2b2d; --pill-fg: #b2c5c3;
  --method-get: #5cc2dc; --method-post: #48bb78; --method-put: #ecc94b; --method-del: #fc8181;
}
:root[data-theme="light"] {
  --bg: #f3f5f5; --fg: #13201f; --dim: #55615f; --line: #d9dfde; --card: #ffffff; --code: #e9eeed;
  --link: #11627a;
  --pill-bg: #e2e8e7; --pill-fg: #2c4240;
  --method-get: #16748d; --method-post: #2a8a5b; --method-put: #b27318; --method-del: #b73e34;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.6 var(--sans); }
a { color: inherit; }
:focus-visible { outline: 2px solid var(--teal-2); outline-offset: 3px; border-radius: 3px; }
.wrap { max-width: 72rem; margin: 0 auto; padding: 0 1.5rem; }
.top { background: var(--ink); color: var(--cream); }
nav.bar { display: flex; align-items: center; gap: 1.5rem; height: 64px; }
nav.bar .brand { display: flex; align-items: center; gap: .6rem; font-weight: 650; text-decoration: none; letter-spacing: -.01em; }
nav.bar .links { display: flex; gap: 1.25rem; margin-left: auto; font-size: .9rem; }
nav.bar .links a { text-decoration: none; opacity: .78; }
nav.bar .links a:hover, nav.bar .links a[aria-current] { opacity: 1; }
nav.bar .links a[aria-current] { box-shadow: inset 0 -2px 0 #6cc3d8; }
@media (max-width: 720px) { nav.bar .links a.opt { display: none; } }
`;

/** The top bar. `current` marks the section the page belongs to. */
export function topBar(root: string, current: "home" | "docs" | "reference"): string {
  const link = (href: string, label: string, key: string, opt = false) =>
    `<a${opt ? ' class="opt"' : ""} href="${root}${href}"${current === key ? ' aria-current="page"' : ""}>${label}</a>`;
  return `<div class="top"><div class="wrap"><nav class="bar" aria-label="Site">
  <a class="brand" href="${root}index.html"><img src="${root}favicon.svg" alt="" width="28" height="28">Sjel</a>
  <div class="links">
    ${link("docs/index.html", "Docs", "docs")}
    ${link("reference/index.html", "Reference", "reference")}
    <a href="${REPO}">GitHub</a>
  </div>
</nav></div></div>`;
}

const PAGE_CSS = `
main { padding: 2.5rem 0 4rem; }
.layout { display: grid; grid-template-columns: 15rem minmax(0, 1fr) 13rem; gap: 2.5rem; }
.layout.two { grid-template-columns: 15rem minmax(0, 1fr); }
@media (max-width: 1100px) { .layout, .layout.two { grid-template-columns: 13rem minmax(0, 1fr); } .toc { display: none; } }
@media (max-width: 760px) { .layout, .layout.two { grid-template-columns: minmax(0, 1fr); } .side { order: 2; } }
.side, .toc { font-size: .88rem; }
.side nav, .toc nav { position: sticky; top: 1.5rem; }
.side h4, .toc h4 { margin: 1.2rem 0 .4rem; font: 500 .72rem var(--mono); text-transform: uppercase; letter-spacing: .07em; color: var(--dim); }
.side h4:first-child, .toc h4:first-child { margin-top: 0; }
.side ul, .toc ul { list-style: none; margin: 0; padding: 0; }
.side a, .toc a { display: block; padding: .22rem .6rem; border-radius: 4px; text-decoration: none; color: var(--dim); }
.side a:hover, .toc a:hover { color: var(--fg); background: var(--card); }
.side a[aria-current] { color: var(--fg); background: var(--card); box-shadow: inset 2px 0 0 var(--teal); font-weight: 550; }
.toc a.sub { padding-left: 1.2rem; }

article { min-width: 0; }
article > :first-child { margin-top: 0; }
article h1 { font-size: 2.1rem; letter-spacing: -.025em; line-height: 1.15; margin: 0 0 1rem; }
article h2 { font-size: 1.4rem; letter-spacing: -.015em; margin: 2.4rem 0 .7rem; padding-top: .4rem; border-top: 1px solid var(--line); }
article h3 { font-size: 1.08rem; margin: 1.8rem 0 .5rem; }
article h4 { font-size: .98rem; margin: 1.4rem 0 .4rem; }
article p, article li { max-width: 46rem; }
article a { color: var(--link); text-underline-offset: 2px; }
article .anchor { opacity: 0; margin-left: .35rem; text-decoration: none; color: var(--dim); }
article h2:hover .anchor, article h3:hover .anchor { opacity: 1; }
article code { font: .86em var(--mono); background: var(--code); padding: .1rem .35rem; border-radius: 4px; overflow-wrap: anywhere; }
article pre { background: var(--ink); color: #d5e2e0; padding: 1.1rem 1.25rem; border-radius: 6px; overflow-x: auto; font: .84rem/1.55 var(--mono); position: relative; border: 1px solid rgba(255,255,255,0.06); }
article pre code { background: none; padding: 0; color: inherit; font: inherit; }
pre .copy-btn { position: absolute; top: .6rem; right: .6rem; background: rgba(255,255,255,0.08); border: 1px solid rgba(255,255,255,0.15); color: #a0b2b0; border-radius: 4px; padding: .25rem .45rem; cursor: pointer; transition: all .15s; font: 500 .72rem var(--mono); line-height: 1; }
pre .copy-btn:hover { background: rgba(255,255,255,0.18); color: #fff; }
pre .copy-btn.copied { background: #2a8a5b; color: #fff; border-color: #2a8a5b; }
article blockquote { margin: 1rem 0; padding: .6rem 1rem; border-left: 3px solid var(--teal); background: var(--card); color: var(--dim); }
article table, .scroll table { width: 100%; border-collapse: collapse; font-size: .9rem; }
article table { display: block; overflow-x: auto; }
article th, article td, .scroll th, .scroll td { text-align: left; padding: .5rem .75rem; border-bottom: 1px solid var(--line); vertical-align: top; }
article thead th, .scroll thead th { font: 500 .72rem var(--mono); text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
article hr { border: 0; border-top: 1px solid var(--line); margin: 2rem 0; }
article img { max-width: 100%; }

.tagline, .blurb { color: var(--dim); max-width: 46rem; }
.tagline { font-size: 1.05rem; margin: 0 0 1rem; }
.blurb { font-size: .92rem; margin: 0 0 1rem; }
.count { margin-left: .4rem; padding: .05rem .45rem; border-radius: 4px; background: var(--code); font: 400 .75rem var(--mono); color: var(--dim); vertical-align: middle; }
.scroll { overflow-x: auto; border: 1px solid var(--line); border-radius: 8px; background: var(--card); margin: .5rem 0 1rem; }
.scroll tbody tr:last-child th, .scroll tbody tr:last-child td { border-bottom: 0; }
.scroll tbody th { white-space: nowrap; font-weight: 600; }
.scroll code { font: .82rem var(--mono); }
.totals { display: flex; flex-wrap: wrap; gap: .5rem; margin: 1rem 0 0; padding: 0; list-style: none; }
.totals li { padding: .3rem .7rem; border: 1px solid var(--line); border-radius: 999px; background: var(--card); font-size: .82rem; color: var(--dim); }
.totals strong { color: var(--fg); font-variant-numeric: tabular-nums; }
.cards { list-style: none; margin: 0; padding: 0; display: grid; gap: .6rem; grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr)); }
.cards a { display: block; height: 100%; padding: .8rem 1rem; border: 1px solid var(--line); border-radius: 6px; background: var(--card); text-decoration: none; transition: border-color .15s; }
.cards a:hover { border-color: var(--teal); }
.cards strong { display: block; font-family: var(--mono); font-size: .9rem; }
.cards span { font-size: .82rem; color: var(--dim); }
.pill { display: inline-flex; align-items: center; gap: .3rem; font: 500 .72rem var(--mono); padding: .15rem .5rem; border-radius: 999px; background: var(--pill-bg); color: var(--pill-fg); border: 1px solid var(--line); }
.pill.proc { border-color: rgba(42, 138, 91, 0.4); color: var(--method-post); }
.pill.cont { border-color: rgba(22, 116, 141, 0.4); color: var(--method-get); }
.pill.data { border-color: rgba(178, 115, 24, 0.4); color: var(--method-put); }
.pins { list-style: none; padding: 0; display: flex; flex-wrap: wrap; gap: .4rem; }
.pins li { display: flex; gap: .4rem; align-items: center; padding: .2rem .55rem; border: 1px solid var(--line); border-radius: 4px; background: var(--card); font-size: .84rem; }
.pins code { background: none; padding: 0; color: var(--dim); font-size: .75rem; }
.upstream-list { margin: 0; padding: 0; list-style: none; display: grid; gap: .3rem; grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr)); font: .84rem var(--mono); }
.scroll table, article .scroll table { display: table; }
td.route { white-space: nowrap; }
.method { display: inline-block; min-width: 3.8rem; font: 600 .72rem var(--mono); text-transform: uppercase; }
.method.get { color: var(--method-get); }
.method.post { color: var(--method-post); }
.method.put, .method.patch { color: var(--method-put); }
.method.delete { color: var(--method-del); }
.schema { font: .7rem var(--mono); color: var(--dim); border: 1px solid var(--line); border-radius: 3px; padding: 0 .3rem; }
.note { padding: .7rem 1rem; border-left: 3px solid var(--teal); background: var(--card); font-size: .9rem; }
footer.site { border-top: 1px solid var(--line); padding: 1.5rem 0 3rem; font-size: .84rem; color: var(--dim); }
footer.site code { font: .8rem var(--mono); }
`;

export interface PageOptions {
  title: string;
  description: string;
  /** Prefix back to the site root: "" at the root, "../" one level down. */
  root: string;
  current: "home" | "docs" | "reference";
  body: string;
  /** Left column. With `toc`, the page gets three columns. */
  sidebar?: string;
  toc?: string;
  footer?: string;
}

export function page(opts: PageOptions): string {
  const main = opts.sidebar
    ? `<div class="layout${opts.toc ? "" : " two"}">
  <aside class="side">${opts.sidebar}</aside>
  <article>${opts.body}</article>
  ${opts.toc ? `<aside class="toc">${opts.toc}</aside>` : ""}
</div>`
    : `<article>${opts.body}</article>`;
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(opts.title)}</title>
<meta name="description" content="${esc(opts.description)}">
<link rel="icon" href="${opts.root}favicon.svg" type="image/svg+xml">
<style>${TOKENS}${PAGE_CSS}</style>
</head>
<body>
${topBar(opts.root, opts.current)}
<main><div class="wrap">
${main}
</div></main>
<footer class="site"><div class="wrap">${opts.footer ?? ""}</div></footer>
</body>
</html>
`;
}
