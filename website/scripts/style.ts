// website/scripts/style.ts — the shell every page shares: tokens, the top bar, and the layouts
// for docs (sidebar, article, on-this-page) and reference pages. Inlined into each page, so a
// page saved to disk still renders.

export const REPO = "https://github.com/larsboes/Sjel";

export const GITHUB_ICON = `<svg class="icon github" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73 0.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27 0.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"></path></svg>`;

export function esc(value: unknown): string {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

/** Theme bootstrap script: runs in <head> synchronously to prevent theme flash. */
export const THEME_SCRIPT = `<script>
(function() {
  try {
    var t = localStorage.getItem("theme");
    if (t === "dark" || t === "light") {
      document.documentElement.setAttribute("data-theme", t);
    }
  } catch (e) {}
})();
</script>`;

/** Theme control component: System / Light / Dark segmented toggle with keyboard shortcut hint. */
export function themeControl(): string {
  return `<div class="theme-ctrl-wrap">
  <div class="theme-ctrl" role="group" aria-label="Color theme">
    <button type="button" class="theme-btn" data-theme-val="system" aria-label="System theme" title="System theme (T to cycle)">
      <svg class="icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
    </button>
    <button type="button" class="theme-btn" data-theme-val="light" aria-label="Light theme" title="Light theme (T to cycle)">
      <svg class="icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>
    </button>
    <button type="button" class="theme-btn" data-theme-val="dark" aria-label="Dark theme" title="Dark theme (T to cycle)">
      <svg class="icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>
    </button>
  </div>
  <kbd class="theme-kbd" title="Focus this control, then press T to cycle theme" aria-hidden="true">T</kbd>
  <span class="theme-toast" role="status" aria-live="polite"></span>
</div>`;
}

/** Theme initialization, persistence, and 'T' keyboard shortcut handler. */
export const THEME_INIT_SCRIPT = `<script>
(function() {
  var btns = document.querySelectorAll(".theme-btn");
  var toast = document.querySelector(".theme-toast");
  var toastTimer = null;
  function showToast(msg) {
    if (!toast) return;
    toast.textContent = msg;
    toast.classList.add("visible");
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(function() {
      toast.classList.remove("visible");
    }, 1100);
  }
  function update(val) {
    for (var i = 0; i < btns.length; i++) {
      var b = btns[i];
      var active = b.getAttribute("data-theme-val") === val;
      b.setAttribute("aria-pressed", active ? "true" : "false");
      if (active) b.classList.add("active"); else b.classList.remove("active");
    }
  }
  function setTheme(val, announce) {
    current = val;
    if (val === "system") {
      try { localStorage.removeItem("theme"); } catch (e) {}
      document.documentElement.removeAttribute("data-theme");
      if (announce) showToast("Theme: System");
    } else {
      try { localStorage.setItem("theme", val); } catch (e) {}
      document.documentElement.setAttribute("data-theme", val);
      if (announce) showToast("Theme: " + (val === "dark" ? "Dark" : "Light"));
    }
    update(val);
  }
  var current = "system";
  try {
    var stored = localStorage.getItem("theme");
    if (stored === "dark" || stored === "light") current = stored;
  } catch (e) {}
  update(current);
  for (var j = 0; j < btns.length; j++) {
    (function(btn) {
      btn.addEventListener("click", function() {
        setTheme(btn.getAttribute("data-theme-val"), false);
      });
    })(btns[j]);
  }
  window.addEventListener("keydown", function(e) {
    if (e.key !== "t" && e.key !== "T") return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    var el = document.activeElement;
    var group = document.querySelector(".theme-ctrl");
    if (!group || !group.contains(el)) return;
    var order = ["system", "light", "dark"];
    var next = order[(order.indexOf(current) + 1) % order.length];
    e.preventDefault();
    setTheme(next, true);
  });
})();
</script>`;

export const TOKENS = `
:root {
  color-scheme: light dark;
  --bg: #ffffff;
  --fg: #132022;
  --dim: #546466;
  --dim-subtle: #859597;
  --line: #e3e8e9;
  --line-strong: #16748d;
  --card: #ffffff;
  --card-hover: #f3f8f9;
  --code: #edf3f4;
  --code-fg: #132022;
  --term-bg: #0b1517;
  --term-fg: #e2eeea;
  --term-dim: #aabbbc;
  --teal: #16748d;
  --teal-hover: #135d72;
  --teal-light: #2a93ac;
  --teal-bright: #6cc3d8;
  --teal-tint: rgba(22, 116, 141, 0.08);
  --teal-glow: rgba(22, 116, 141, 0.22);
  --link: #16748d;
  --link-hover: #135d72;
  --pill-bg: #edf3f4;
  --pill-fg: #16748d;
  --pill-border: #d4e0e2;
  --focus: #16748d;
  --top-bg: #ffffff;
  --top-fg: #132022;
  --top-line: #e3e8e9;
  --method-get: #16748d;
  --method-post: #2a8a5b;
  --method-put: #b27318;
  --method-del: #b73e34;
  --sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, sans-serif;
  --mono: ui-monospace, "SF Mono", SFMono-Regular, Menlo, monospace;
  --display: ui-serif, "New York", "Iowan Old Style", Charter, Georgia, serif;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #0b1214;
    --fg: #e6eded;
    --dim: #8fa2a3;
    --dim-subtle: #5f7475;
    --line: #1c2b2e;
    --line-strong: #6cc3d8;
    --card: #111d20;
    --card-hover: #17272b;
    --code: #162428;
    --code-fg: #e6eded;
    --term-bg: #070c0d;
    --term-fg: #e6eded;
    --term-dim: #aabbbc;
    --teal: #6cc3d8;
    --teal-hover: #8fd6e8;
    --teal-light: #5cc2dc;
    --teal-bright: #6cc3d8;
    --teal-tint: rgba(108, 195, 216, 0.12);
    --teal-glow: rgba(108, 195, 216, 0.28);
    --link: #6cc3d8;
    --link-hover: #8fd6e8;
    --pill-bg: #162428;
    --pill-fg: #6cc3d8;
    --pill-border: #22373c;
    --focus: #6cc3d8;
    --top-bg: #0b1214;
    --top-fg: #e6eded;
    --top-line: #1c2b2e;
    --method-get: #6cc3d8;
    --method-post: #48bb78;
    --method-put: #ecc94b;
    --method-del: #fc8181;
  }
}
:root[data-theme="dark"] {
  --bg: #0b1214;
  --fg: #e6eded;
  --dim: #8fa2a3;
  --dim-subtle: #5f7475;
  --line: #1c2b2e;
  --line-strong: #6cc3d8;
  --card: #111d20;
  --card-hover: #17272b;
  --code: #162428;
  --code-fg: #e6eded;
  --term-bg: #070c0d;
  --term-fg: #e6eded;
  --term-dim: #aabbbc;
  --teal: #6cc3d8;
  --teal-hover: #8fd6e8;
  --teal-light: #5cc2dc;
  --teal-bright: #6cc3d8;
  --teal-tint: rgba(108, 195, 216, 0.12);
  --teal-glow: rgba(108, 195, 216, 0.28);
  --link: #6cc3d8;
  --link-hover: #8fd6e8;
  --pill-bg: #162428;
  --pill-fg: #6cc3d8;
  --pill-border: #22373c;
  --focus: #6cc3d8;
  --top-bg: #0b1214;
  --top-fg: #e6eded;
  --top-line: #1c2b2e;
  --method-get: #6cc3d8;
  --method-post: #48bb78;
  --method-put: #ecc94b;
  --method-del: #fc8181;
}
:root[data-theme="light"] {
  --bg: #ffffff;
  --fg: #132022;
  --dim: #546466;
  --dim-subtle: #859597;
  --line: #e3e8e9;
  --line-strong: #16748d;
  --card: #ffffff;
  --card-hover: #f3f8f9;
  --code: #edf3f4;
  --code-fg: #132022;
  --term-bg: #0b1517;
  --term-fg: #e2eeea;
  --term-dim: #aabbbc;
  --teal: #16748d;
  --teal-hover: #135d72;
  --teal-light: #2a93ac;
  --teal-bright: #6cc3d8;
  --teal-tint: rgba(22, 116, 141, 0.08);
  --teal-glow: rgba(22, 116, 141, 0.22);
  --link: #16748d;
  --link-hover: #135d72;
  --pill-bg: #edf3f4;
  --pill-fg: #16748d;
  --pill-border: #d4e0e2;
  --focus: #16748d;
  --top-bg: #ffffff;
  --top-fg: #132022;
  --top-line: #e3e8e9;
  --method-get: #16748d;
  --method-post: #2a8a5b;
  --method-put: #b27318;
  --method-del: #b73e34;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg); font: 400 16px/1.6 var(--sans); -webkit-font-smoothing: antialiased; -moz-osx-font-smoothing: grayscale; }
a { color: inherit; }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; border-radius: 2px; }
.wrap { max-width: 96rem; margin: 0 auto; padding: 0 clamp(1.25rem, 3vw, 3rem); }
.top { background: var(--top-bg); color: var(--top-fg); border-bottom: 1px solid var(--top-line); }
nav.bar { display: flex; align-items: center; gap: 1.5rem; height: 64px; }
nav.bar .brand { display: flex; align-items: center; gap: .6rem; font-weight: 600; text-decoration: none; letter-spacing: -.01em; color: var(--fg); }
nav.bar .brand img { border-radius: 4px; display: block; }
nav.bar .links { display: flex; align-items: center; gap: 1.25rem; margin-left: auto; font-size: .9rem; }
nav.bar .links a { text-decoration: none; color: var(--dim); display: inline-flex; align-items: center; gap: .4rem; transition: color .15s; }
nav.bar .links a:hover { color: var(--teal); }
nav.bar .links a[aria-current] { color: var(--teal); box-shadow: inset 0 -2px 0 var(--teal); font-weight: 600; }
.icon-link { display: inline-flex; align-items: center; justify-content: center; color: var(--dim); text-decoration: none; transition: color .15s; }
.icon-link:hover { color: var(--teal); }
.icon { display: inline-block; vertical-align: middle; }

/* Theme switcher segmented control & keyboard shortcut hint */
.theme-ctrl-wrap { display: inline-flex; align-items: center; gap: .35rem; position: relative; }
.theme-ctrl { display: inline-flex; align-items: center; padding: 2px; border: 1px solid var(--line); border-radius: 4px; background: var(--card); gap: 1px; }
.theme-btn { display: inline-flex; align-items: center; justify-content: center; width: 24px; height: 24px; padding: 0; border: none; background: transparent; color: var(--dim); border-radius: 2px; cursor: pointer; transition: color .15s, background-color .15s; }
.theme-btn:hover { color: var(--teal); }
.theme-btn.active, .theme-btn[aria-pressed="true"] { background: var(--term-bg); color: var(--term-fg); box-shadow: inset 0 0 0 1px var(--teal); }
.theme-btn:focus-visible { outline: 2px solid var(--focus); outline-offset: 1px; }
.theme-kbd {
  font: 500 .68rem var(--mono);
  padding: .15rem .35rem;
  border: 1px solid var(--line);
  border-radius: 2px;
  background: var(--card);
  color: var(--dim);
  user-select: none;
  line-height: 1;
}
@media (max-width: 600px) { .theme-kbd { display: none; } }
.theme-toast {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  white-space: nowrap;
  font: 500 .74rem var(--mono);
  padding: .25rem .6rem;
  background: var(--term-bg);
  color: var(--term-fg);
  border: 1px solid var(--teal);
  border-radius: 2px;
  opacity: 0;
  pointer-events: none;
  transition: opacity .15s ease, transform .15s ease;
  transform: translateY(-2px);
  z-index: 100;
  box-shadow: 0 4px 12px var(--teal-glow);
}
.theme-toast.visible { opacity: 1; transform: translateY(0); }

/* Blueprint corner tick marks */
.blueprint { position: relative; }
.blueprint::before, .blueprint::after {
  content: "+";
  position: absolute;
  font: 400 12px/1 var(--mono);
  color: var(--dim-subtle);
  pointer-events: none;
  user-select: none;
  z-index: 2;
}
.blueprint::before { top: -6px; left: -4px; }
.blueprint::after { bottom: -6px; right: -4px; }

@media (max-width: 900px) {
  nav.bar { height: auto; min-height: 64px; padding: .75rem 0; flex-wrap: wrap; gap: .75rem; }
  nav.bar .links { gap: .85rem; font-size: .84rem; flex-wrap: wrap; justify-content: flex-end; }
}
@media (max-width: 480px) {
  nav.bar .brand { font-size: .95rem; }
  nav.bar .links { gap: .6rem; font-size: .8rem; }
}
`;

/** The top bar. `current` marks the section the page belongs to. */
export function topBar(root: string, current: "home" | "docs" | "reference" | "roadmap", extraLinks = ""): string {
  const link = (href: string, label: string, key: string, opt = false) =>
    `<a${opt ? ' class="opt"' : ""} href="${root}${href}"${current === key ? ' aria-current="page"' : ""}>${label}</a>`;
  return `<header class="top"><div class="wrap"><nav class="bar" aria-label="Site">
  <a class="brand" href="${root}index.html"><img src="${root}favicon.svg" alt="" width="28" height="28">Sjel</a>
  <div class="links">
    ${extraLinks}
    ${link("docs/index.html", "Docs", "docs")}
    ${link("reference/index.html", "Reference", "reference")}
    ${link("roadmap/index.html", "Roadmap", "roadmap")}
    <a href="${REPO}" class="icon-link" aria-label="GitHub repository" title="GitHub">${GITHUB_ICON}</a>
    ${themeControl()}
  </div>
</nav></div></header>`;
}

const PAGE_CSS = `
main { padding: 2.5rem 0 4rem; }
.layout { display: grid; grid-template-columns: 17rem minmax(0, 1fr) 15rem; gap: 3.5rem; }
.layout.two { display: grid; grid-template-columns: 17rem minmax(0, 1fr); gap: 3.5rem; }
@media (max-width: 1200px) { .layout { grid-template-columns: 15rem minmax(0, 1fr) 13rem; gap: 2rem; } }
@media (max-width: 980px) { .layout, .layout.two { grid-template-columns: 14rem minmax(0, 1fr); gap: 2rem; } .toc { display: none; } }
@media (max-width: 760px) { .layout, .layout.two { grid-template-columns: minmax(0, 1fr); gap: 2rem; } .side { order: 2; } }
.side, .toc { font-size: .88rem; }
.side nav, .toc nav { position: sticky; top: 1.5rem; }
.side h4, .toc h4 { margin: 1.5rem 0 .45rem; font: 600 .74rem var(--mono); text-transform: uppercase; letter-spacing: .08em; color: var(--dim); }
.side h4:first-child, .toc h4:first-child { margin-top: 0; }
.side ul, .toc ul { list-style: none; margin: 0; padding: 0; }
.side a, .toc a { display: block; padding: .32rem .65rem; border-radius: 2px; text-decoration: none; color: var(--dim); transition: color .15s, background-color .15s; }
.side a:hover, .toc a:hover { color: var(--teal); background: var(--card-hover); }
.side a[aria-current] { color: var(--teal); background: var(--card); border: 1px solid var(--line); border-left: 2px solid var(--teal); font-weight: 500; }
.toc ul { border-left: 1px solid var(--line); padding-left: .5rem; }
.toc a { padding: .2rem .5rem; border-radius: 2px; font-size: .84rem; }
.toc a:hover { color: var(--teal); background: transparent; }
.toc a.sub { padding-left: 1.2rem; }

article { min-width: 0; width: 100%; }
article > :first-child { margin-top: 0; }
article h1 { font: 400 clamp(2.25rem, 4vw, 3.2rem)/1.08 var(--display); letter-spacing: -.025em; margin: 0 0 1rem; color: var(--fg); }
article h2 { font-size: clamp(1.35rem, 2vw, 1.65rem); font-weight: 500; letter-spacing: -.02em; margin: 2.5rem 0 .8rem; padding-top: .5rem; border-top: 1px solid var(--line); color: var(--fg); }
article h3 { font-size: 1.15rem; font-weight: 500; margin: 1.8rem 0 .5rem; color: var(--fg); }
article h4 { font-size: 1rem; font-weight: 500; margin: 1.4rem 0 .4rem; color: var(--fg); }
article p, article li { max-width: 68rem; line-height: 1.7; font-size: 1.02rem; color: var(--fg); }
article a { color: var(--link); text-decoration: underline; text-underline-offset: 3px; text-decoration-color: var(--teal-tint); transition: color .15s, text-decoration-color .15s; }
article a:hover { color: var(--link-hover); text-decoration-color: var(--link); }
article .anchor { opacity: 0; margin-left: .35rem; text-decoration: none; color: var(--dim); }
article h2:hover .anchor, article h3:hover .anchor { opacity: 1; }
article code { font: .86em var(--mono); background: var(--code); color: var(--code-fg); padding: .1rem .35rem; border-radius: 2px; overflow-wrap: anywhere; }
article pre { background: var(--term-bg); color: var(--term-fg); padding: 1.1rem 1.25rem; border-radius: 2px; overflow-x: auto; font: .84rem/1.55 var(--mono); position: relative; border: 1px solid var(--line); width: 100%; }
article pre code { background: none; padding: 0; color: inherit; font: inherit; }
pre .copy-btn { position: absolute; top: .6rem; right: .6rem; background: var(--card); border: 1px solid var(--line); color: var(--dim); border-radius: 2px; padding: .25rem .45rem; cursor: pointer; transition: color .15s, background-color .15s, border-color .15s; font: 500 .72rem var(--mono); line-height: 1; }
pre .copy-btn:hover { background: var(--card-hover); color: var(--teal); border-color: var(--teal); }
pre .copy-btn.copied { background: var(--teal); color: #ffffff; border-color: var(--teal); }
article blockquote { margin: 1rem 0; padding: .6rem 1rem; border-left: 3px solid var(--teal); background: var(--card); color: var(--dim); max-width: 68rem; }
article table, .scroll table { width: 100%; border-collapse: collapse; font-size: .9rem; }
article table { display: block; overflow-x: auto; -webkit-overflow-scrolling: touch; }
article th, article td, .scroll th, .scroll td { text-align: left; padding: .6rem .85rem; border-bottom: 1px solid var(--line); vertical-align: top; }
article thead th, .scroll thead th { font: 500 .72rem var(--mono); text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
article hr { border: 0; border-top: 1px solid var(--line); margin: 2rem 0; }
article img { max-width: 100%; height: auto; }

.tagline, .blurb { color: var(--dim); max-width: 68rem; }
.tagline { font-size: 1.05rem; margin: 0 0 1rem; }
.blurb { font-size: .94rem; margin: 0 0 1rem; }
.count { margin-left: .4rem; padding: .05rem .45rem; border-radius: 2px; background: var(--code); font: 400 .75rem var(--mono); color: var(--dim); vertical-align: middle; }
.scroll { overflow-x: auto; -webkit-overflow-scrolling: touch; border: 1px solid var(--line); border-radius: 2px; background: var(--card); margin: .5rem 0 1rem; width: 100%; }
.scroll tbody tr:last-child th, .scroll tbody tr:last-child td { border-bottom: 0; }
.scroll tbody th { white-space: nowrap; font-weight: 500; }
.scroll code { font: .82rem var(--mono); }
.totals { display: flex; flex-wrap: wrap; gap: .5rem; margin: 1rem 0 0; padding: 0; list-style: none; }
.totals li { padding: .3rem .7rem; border: 1px solid var(--line); border-radius: 2px; background: var(--card); font-size: .82rem; color: var(--dim); }
.totals strong { color: var(--fg); font-variant-numeric: tabular-nums; }
.cards { list-style: none; margin: 0; padding: 0; display: grid; gap: .75rem; grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr)); width: 100%; }
.cards a { display: block; height: 100%; padding: .9rem 1.1rem; border: 1px solid var(--line); border-radius: 2px; background: var(--card); text-decoration: none; color: inherit; transition: border-color .16s ease, background-color .16s ease, transform .16s ease; }
.cards a:hover { border-color: var(--teal); background: var(--card-hover); transform: translateY(-1px); }
.cards strong { display: block; font-family: var(--mono); font-size: .9rem; color: var(--fg); transition: color .15s; }
.cards a:hover strong { color: var(--teal); }
.cards span { font-size: .82rem; color: var(--dim); }
.pill { display: inline-flex; align-items: center; gap: .3rem; font: 500 .72rem var(--mono); padding: .15rem .5rem; border-radius: 2px; background: var(--pill-bg); color: var(--pill-fg); border: 1px solid var(--pill-border); }
.pill.proc { border-color: rgba(42, 138, 91, 0.35); color: var(--method-post); }
.pill.cont { border-color: rgba(22, 116, 141, 0.35); color: var(--method-get); }
.pill.data { border-color: rgba(178, 115, 24, 0.35); color: var(--method-put); }
.pins { list-style: none; padding: 0; display: flex; flex-wrap: wrap; gap: .4rem; }
.pins li { display: flex; gap: .4rem; align-items: center; padding: .2rem .55rem; border: 1px solid var(--line); border-radius: 2px; background: var(--card); font-size: .84rem; }
.pins code { background: none; padding: 0; color: var(--dim); font-size: .75rem; }
.upstream-list { margin: 0; padding: 0; list-style: none; display: grid; gap: .4rem; grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr)); font: .84rem var(--mono); }
.scroll table, article .scroll table { display: table; width: 100%; }
td.route { white-space: nowrap; }
.method { display: inline-block; min-width: 3.8rem; font: 600 .72rem var(--mono); text-transform: uppercase; }
.method.get { color: var(--method-get); }
.method.post { color: var(--method-post); }
.method.put, .method.patch { color: var(--method-put); }
.method.delete { color: var(--method-del); }
.schema { font: .7rem var(--mono); color: var(--dim); border: 1px solid var(--line); border-radius: 2px; padding: 0 .3rem; }
.note { padding: .7rem 1rem; border-left: 3px solid var(--teal); background: var(--card); font-size: .9rem; max-width: 68rem; color: var(--dim); }
.crumbs { display: flex; align-items: center; flex-wrap: wrap; gap: .35rem; font: 500 .82rem var(--sans); color: var(--dim); margin: 0 0 1.25rem; }
.crumbs a { color: var(--dim); text-decoration: none; transition: color .15s; }
.crumbs a:hover { color: var(--teal); }
.crumbs .sep { color: var(--dim-subtle); opacity: .6; font-size: .75rem; user-select: none; }
.crumbs .current { color: var(--fg); font-weight: 500; }

.lede-docs { font-size: 1.2rem; color: var(--dim); line-height: 1.6; margin: 0 0 2rem; max-width: 64rem; }

.action-card {
  display: flex; align-items: center; justify-content: space-between; gap: 1.5rem;
  padding: 1.1rem 1.4rem; border: 1px solid var(--line); border-radius: 2px;
  background: var(--card); text-decoration: none; margin: .85rem 0 1.25rem;
  transition: border-color .16s ease, background-color .16s ease, transform .16s ease; max-width: 68rem;
}
.action-card:hover { border-color: var(--teal); background: var(--card-hover); transform: translateY(-1px); }
.action-card .card-main { display: flex; flex-direction: column; gap: .2rem; }
.action-card strong { font-size: 1.02rem; color: var(--fg); font-weight: 500; transition: color .15s; }
.action-card:hover strong { color: var(--teal); }
.action-card span.desc { font-size: .88rem; color: var(--dim); line-height: 1.45; }
.action-card .chevron { font-size: 1.2rem; color: var(--dim); transition: transform .15s, color .15s; font-weight: 500; flex-shrink: 0; }
.action-card:hover .chevron { color: var(--teal); transform: translateX(3px); }

.section-grid {
  display: grid; grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr)); gap: .85rem;
  margin: 1rem 0 2.25rem; max-width: 68rem;
}
.section-card {
  display: flex; align-items: center; justify-content: space-between;
  padding: .95rem 1.25rem; border: 1px solid var(--line); border-radius: 2px;
  background: var(--card); text-decoration: none; font-weight: 500; font-size: .95rem; color: var(--fg);
  transition: border-color .16s ease, background-color .16s ease, color .16s ease, transform .16s ease;
}
.section-card:hover { border-color: var(--teal); color: var(--teal); background: var(--card-hover); transform: translateY(-1px); }
.section-card .chevron { color: var(--dim); transition: transform .15s, color .15s; font-weight: 500; }
.section-card:hover .chevron { color: var(--teal); transform: translateX(3px); }

.guide-grid {
  display: grid; grid-template-columns: repeat(auto-fill, minmax(20rem, 1fr)); gap: 1rem;
  margin: 1.25rem 0 2.5rem; max-width: 68rem;
}
.guide-card {
  display: flex; flex-direction: column; justify-content: space-between;
  padding: 1.2rem 1.35rem; border: 1px solid var(--line); border-radius: 2px;
  background: var(--card); transition: border-color .16s ease, background-color .16s ease, transform .16s ease;
}
.guide-card:hover { border-color: var(--teal); background: var(--card-hover); transform: translateY(-1px); }
.guide-tag {
  align-self: flex-start; font: 500 .7rem var(--mono); text-transform: uppercase;
  letter-spacing: .06em; padding: .15rem .45rem; border-radius: 2px;
  background: var(--teal-tint); color: var(--teal); margin-bottom: .6rem;
}
.guide-card h4 { font-size: 1.05rem; font-weight: 500; margin: 0 0 .85rem; line-height: 1.35; color: var(--fg); }
.guide-actions {
  display: flex; flex-direction: column; gap: .45rem; margin-top: auto;
  padding-top: .6rem; border-top: 1px solid var(--line);
}
.btn-guide {
  display: inline-flex; align-items: center; justify-content: space-between;
  font-size: .86rem; text-decoration: none; padding: .35rem 0; color: var(--fg);
  transition: color .15s;
}
.btn-guide.primary { font-weight: 500; color: var(--teal); text-decoration: underline; text-underline-offset: 3px; }
.btn-guide.primary:hover { color: var(--teal-hover); }
.btn-guide.secondary { color: var(--dim); font-size: .82rem; }
.btn-guide.secondary:hover { color: var(--fg); }

.doc-pager {
  display: grid; grid-template-columns: 1fr 1fr; gap: 1.25rem;
  margin-top: 3.5rem; padding-top: 1.75rem; border-top: 1px solid var(--line); max-width: 68rem;
}
@media (max-width: 640px) { .doc-pager { grid-template-columns: 1fr; } }
.pager-card {
  display: flex; flex-direction: column; gap: .3rem; padding: 1rem 1.25rem;
  border: 1px solid var(--line); border-radius: 2px; background: var(--card);
  text-decoration: none; transition: border-color .16s ease, background-color .16s ease;
}
.pager-card:hover { border-color: var(--teal); background: var(--card-hover); }
.pager-card:hover .pager-title { color: var(--teal); }
.pager-card.next { text-align: right; margin-left: auto; width: 100%; }
.pager-card.prev { text-align: left; margin-right: auto; width: 100%; }
.pager-label { font: 500 .72rem var(--mono); text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
.pager-title { font-weight: 500; font-size: .95rem; color: var(--fg); transition: color .15s; }

.roadmap-wrap { max-width: 78rem; margin: 0 auto; }
.roadmap-hero { margin-bottom: 3rem; }
.roadmap-hero h1 { font: 400 clamp(2.4rem, 4.2vw, 3.4rem) var(--display); letter-spacing: -.03em; margin: 0 0 .75rem; color: var(--fg); }
.roadmap-hero p { font-size: 1.2rem; color: var(--dim); max-width: 52rem; line-height: 1.6; margin: 0 0 1rem; }
.roadmap-hero .subtext { font-size: .92rem; color: var(--dim); }
.roadmap-hero .subtext a { color: var(--teal); text-decoration: underline; text-underline-offset: 2px; }

.roadmap-section {
  display: grid; grid-template-columns: 14rem 1fr; gap: 2.5rem;
  padding: 3rem 0; border-top: 1px solid var(--line);
}
@media (max-width: 800px) { .roadmap-section { grid-template-columns: 1fr; gap: 1.25rem; } }

.roadmap-status { position: sticky; top: 2rem; align-self: start; }
.roadmap-status h2 {
  font-size: 1.35rem; font-weight: 500; display: flex; align-items: center;
  gap: .6rem; margin: 0; letter-spacing: -.01em; color: var(--fg);
}

.timeline-group {
  display: grid; grid-template-columns: 14rem 1fr; gap: 2.5rem; margin-bottom: 2.5rem;
}
.timeline-group:last-child { margin-bottom: 0; }
@media (max-width: 800px) { .timeline-group { grid-template-columns: 1fr; gap: .75rem; } }
.timeline-date { font-size: 1.15rem; font-weight: 500; color: var(--fg); padding-top: .4rem; }

.roadmap-cards { display: flex; flex-direction: column; gap: .85rem; }
.roadmap-card {
  background: var(--card); border: 1px solid var(--line); border-radius: 2px;
  padding: 1.15rem 1.35rem; text-decoration: none; color: inherit;
  transition: border-color .16s ease, background-color .16s ease, transform .16s ease;
  display: flex; flex-direction: column; gap: .4rem;
}
.roadmap-card:hover {
  border-color: var(--teal); background: var(--card-hover); transform: translateY(-1px);
}
.roadmap-card-head { display: flex; align-items: center; justify-content: space-between; gap: 1rem; }
.roadmap-card-title { font-weight: 500; font-size: 1.05rem; color: var(--fg); display: inline-flex; align-items: center; gap: .45rem; transition: color .15s; }
.roadmap-card:hover .roadmap-card-title { color: var(--teal); }
.roadmap-card-title .ext { font-size: .8rem; opacity: .5; }
.roadmap-badge {
  font: 500 .72rem var(--mono); padding: .15rem .5rem; border-radius: 2px;
  background: var(--teal-tint); color: var(--teal); border: 1px solid var(--pill-border); white-space: nowrap;
}
.roadmap-card-desc { font-size: .92rem; color: var(--dim); line-height: 1.55; margin: 0; }

footer.site { border-top: 1px solid var(--line); padding: 1.5rem 0 3rem; font-size: .84rem; color: var(--dim); }
footer.site .wrap { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 1rem; }
footer.site code { font: .8rem var(--mono); }
footer.site a { color: var(--dim); text-decoration: none; transition: color .15s; }
footer.site a:hover { color: var(--fg); }

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: .01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: .01ms !important;
    scroll-behavior: auto !important;
  }
}
`;

export interface PageOptions {
  title: string;
  description: string;
  /** Prefix back to the site root: "" at the root, "../" one level down. */
  root: string;
  current: "home" | "docs" | "reference" | "roadmap";
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
${THEME_SCRIPT}
<style>${TOKENS}${PAGE_CSS}</style>
</head>
<body>
${topBar(opts.root, opts.current)}
<main><div class="wrap">
${main}
</div></main>
<footer class="site"><div class="wrap">${opts.footer ?? ""}</div></footer>
${THEME_INIT_SCRIPT}
</body>
</html>
`;
}
