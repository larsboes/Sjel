#!/usr/bin/env bun
// website/scripts/landing.ts — the public landing page: what Sjel is, how it fits together, and
// the way into the reference docs.
//
//   bun website/scripts/landing.ts                write site/index.html
//   bun website/scripts/landing.ts --out <dir>    write <dir>/index.html
//
// The copy follows README.md. Counts and the capability index come from self.json, so a new
// capability appears here without an edit. Self-contained: one HTML file, no script, no font.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { esc, GITHUB_ICON } from "./style.ts";

const SJEL_ROOT = resolve(dirname(new URL(import.meta.url).pathname), "../..");
const REPO = "https://github.com/larsboes/Sjel";

interface Unit { name: string; kind: string; service?: { kind: string; port?: string } }
interface SelfModel { units: Unit[]; upstreams: unknown[] }

/** Life areas for the capability index. A capability not named here lands in "Platform". */
const AREAS: [string, string, string[]][] = [
  ["People & places", "Who someone is, where they live, who is around a trip.",
    ["entities", "entities-google-sync", "entities-sync", "people-registry", "places", "operator-profile"]],
  ["Travel", "Trips, legs, connections, fares and punctuality.",
    ["trips", "traveler", "transit", "punctuality", "punctuality-ingest", "sparpreis-watch", "scouting"]],
  ["Money & home", "Accounts, prices, inventory, wishes and the flat itself.",
    ["finance", "finance-prices", "inventory", "wish-prices", "interior"]],
  ["Time & work", "Calendar, projects, CV and the knowledge base.",
    ["calendar", "projects", "cv", "knowledge-base", "vault"]],
  ["Mail, feed & documents", "Mail triage, reading, clips, documents and media.",
    ["comms", "feed-sweep", "documents", "import", "media", "web-links", "archive", "soundscape"]],
  ["Assistant", "The agent, local models and the agent sandbox.",
    ["assistant", "foundation-models", "agentbox", "packs"]],
];

function capabilityIndex(units: Unit[]): string {
  const caps = units.filter((u) => u.kind === "capability").map((u) => u.name);
  const named = new Set(AREAS.flatMap(([, , names]) => names));
  const areas: [string, string, string[]][] = [
    ...AREAS.map(([t, d, names]) => [t, d, names.filter((n) => caps.includes(n))] as [string, string, string[]]),
    ["Platform", "Storage, backup, devices, host health and the status shell.", caps.filter((n) => !named.has(n))],
  ];
  return areas.filter(([, , names]) => names.length > 0).map(([title, blurb, names]) => `
      <div class="area${title === "Platform" ? " wide" : ""}">
        <h3>${esc(title)} <span class="n">${names.length}</span></h3>
        <p>${esc(blurb)}</p>
        <ul>${names.sort().map((n) => `<li><a href="reference/${esc(n)}.html">${esc(n)}</a></li>`).join("")}</ul>
      </div>`).join("");
}

/** The architecture figure: devices, the node, its gate and the models. */
const ARCHITECTURE = `
<svg class="arch" viewBox="0 0 900 460" role="img" aria-labelledby="arch-title arch-desc">
  <title id="arch-title">How Sjel fits together</title>
  <desc id="arch-desc">The iPhone and Mac apps send requests to one node. The node holds the core and the capabilities. Model calls leave the node only through the data class gate.</desc>
  <defs>
    <pattern id="dots" width="16" height="16" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" class="dot"/></pattern>
    <marker id="tip" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 8 4 0 8z" class="tipfill"/></marker>
  </defs>
  <rect width="900" height="460" fill="url(#dots)"/>

  <g class="device">
    <rect x="24" y="80" width="196" height="110" rx="6"/>
    <rect x="42" y="100" width="30" height="56" rx="5" class="glyph"/><rect x="51" y="105" width="12" height="2" rx="1" class="glyph"/>
    <text x="88" y="118" class="h">iPhone app</text>
    <text x="88" y="140" class="s">GPUI · local copy</text>
    <text x="88" y="158" class="s">signs every request</text>
    <text x="88" y="176" class="s">Wi-Fi or Tailscale</text>
  </g>
  <g class="device">
    <rect x="24" y="270" width="196" height="110" rx="6"/>
    <rect x="44" y="294" width="40" height="28" rx="3" class="glyph"/><rect x="38" y="325" width="52" height="5" rx="2" class="glyph"/>
    <text x="104" y="308" class="h">Mac app</text>
    <text x="104" y="330" class="s">GPUI · native</text>
    <text x="104" y="348" class="s">loopback HTTP</text>
    <text x="104" y="366" class="s">first frame &lt; 10 ms</text>
  </g>

  <path id="p-phone" d="M220 135 C 262 135, 262 190, 304 190" class="wire dashed" marker-end="url(#tip)"/>
  <path id="p-mac" d="M220 325 C 262 325, 262 270, 304 270" class="wire" marker-end="url(#tip)"/>

  <g class="node">
    <rect x="304" y="24" width="316" height="412" rx="8"/>
    <text x="324" y="52" class="h">Node</text>
    <text x="324" y="70" class="s">a Mac or a Linux machine · holds every write</text>
    <rect x="324" y="86" width="276" height="74" rx="5" class="core"/>
    <text x="340" y="110" class="h inv">Core</text>
    <text x="340" y="130" class="s inv">entities · devices · data classes</text>
    <text x="340" y="148" class="s inv">pseudonymizer · SQLite stores</text>
    <g class="chips">
      <rect x="324" y="176" width="132" height="34" rx="4"/><text x="338" y="198">people</text>
      <rect x="468" y="176" width="132" height="34" rx="4"/><text x="482" y="198">travel</text>
      <rect x="324" y="220" width="132" height="34" rx="4"/><text x="338" y="242">money</text>
      <rect x="468" y="220" width="132" height="34" rx="4"/><text x="482" y="242">home</text>
      <rect x="324" y="264" width="132" height="34" rx="4"/><text x="338" y="286">calendar</text>
      <rect x="468" y="264" width="132" height="34" rx="4"/><text x="482" y="286">mail · feed</text>
      <rect x="324" y="308" width="132" height="34" rx="4"/><text x="338" y="330">documents</text>
      <rect x="468" y="308" width="132" height="34" rx="4"/><text x="482" y="330">agent</text>
    </g>
    <text x="324" y="374" class="s">Each capability: its own store and routes.</text>
    <text x="324" y="394" class="s">Private values live in the overlay.</text>
    <text x="324" y="414" class="s">Secrets live in the OS keychain.</text>
  </g>

  <path id="p-model" d="M620 230 L 706 230" class="wire" marker-end="url(#tip)"/>
  <g class="gate">
    <rect x="636" y="196" width="52" height="68" rx="4"/>
    <text x="662" y="226" class="s c">class</text><text x="662" y="242" class="s c">gate</text>
  </g>
  <g class="device">
    <rect x="706" y="146" width="176" height="168" rx="6"/>
    <text x="722" y="174" class="h">Models</text>
    <text x="722" y="198" class="s">1 · a model on the device</text>
    <text x="722" y="216" class="s">2 · a model on the Mac</text>
    <text x="722" y="234" class="s">3 · fixed rules</text>
    <text x="722" y="264" class="s">Cloud gets c0, or c1</text>
    <text x="722" y="280" class="s">as a redacted derivative.</text>
    <text x="722" y="296" class="s">Never c2 or c3.</text>
  </g>

  <g class="pulse" aria-hidden="true">
    <circle r="4"><animateMotion dur="2.6s" repeatCount="indefinite"><mpath href="#p-phone"/></animateMotion></circle>
    <circle r="4"><animateMotion dur="2.2s" repeatCount="indefinite"><mpath href="#p-mac"/></animateMotion></circle>
    <circle r="4"><animateMotion dur="1.8s" repeatCount="indefinite"><mpath href="#p-model"/></animateMotion></circle>
  </g>
</svg>`;

const CSS = `
:root {
  color-scheme: light dark;
  --ink: #0b1d22; --ink-2: #10292f; --teal: #16748d; --teal-2: #1f8fab; --cream: #f7f4ec;
  --bg: #f3f5f5; --fg: #13201f; --dim: #55615f; --line: #d9dfde; --card: #ffffff;
  --pill-bg: #e2e8e7; --pill-fg: #2c4240;
  --method-get: #16748d; --method-post: #2a8a5b; --method-put: #b27318;
  --mono: ui-monospace, "SF Mono", SFMono-Regular, Menlo, monospace;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #0e1517; --fg: #e4eae9; --dim: #93a19f; --line: #22302f; --card: #131d1f;
    --pill-bg: #1c2b2d; --pill-fg: #b2c5c3;
    --method-get: #5cc2dc; --method-post: #48bb78; --method-put: #ecc94b;
  }
}
:root[data-theme="dark"] {
  --bg: #0e1517; --fg: #e4eae9; --dim: #93a19f; --line: #22302f; --card: #131d1f;
  --pill-bg: #1c2b2d; --pill-fg: #b2c5c3;
  --method-get: #5cc2dc; --method-post: #48bb78; --method-put: #ecc94b;
}
:root[data-theme="light"] {
  --bg: #f3f5f5; --fg: #13201f; --dim: #55615f; --line: #d9dfde; --card: #ffffff;
  --pill-bg: #e2e8e7; --pill-fg: #2c4240;
  --method-get: #16748d; --method-post: #2a8a5b; --method-put: #b27318;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg);
  font: 16px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, sans-serif; }
a { color: inherit; }
.wrap { max-width: 96rem; margin: 0 auto; padding: 0 clamp(1.25rem, 3vw, 3rem); }
/* Grid children default to min-width: auto, so one long line would widen the page. */
.hero > *, .split > *, .areas > *, .classes > *, .steps > *, .docs > * { min-width: 0; }
:focus-visible { outline: 2px solid var(--teal-2); outline-offset: 3px; border-radius: 3px; }

.top { background: var(--ink); color: var(--cream); }
nav.bar { display: flex; align-items: center; gap: 1.5rem; height: 64px; }
nav.bar .brand { display: flex; align-items: center; gap: .6rem; font-weight: 650; text-decoration: none; letter-spacing: -.01em; }
nav.bar .links { display: flex; align-items: center; gap: 1.25rem; margin-left: auto; font-size: .9rem; }
nav.bar .links a { text-decoration: none; opacity: .78; }
nav.bar .links a:hover { opacity: 1; }
.icon-link { display: inline-flex; align-items: center; justify-content: center; opacity: .82; text-decoration: none; transition: opacity .15s, transform .15s; }
.icon-link:hover { opacity: 1; transform: translateY(-1px); }
.btn svg, .btn .icon { flex-shrink: 0; }
@media (max-width: 720px) { nav.bar .links a.opt { display: none; } }

.hero { display: grid; grid-template-columns: 1.25fr 1fr; gap: clamp(2rem, 5vw, 4.5rem); align-items: center; padding: 5rem 0 5.5rem; }
@media (max-width: 900px) { .hero { grid-template-columns: 1fr; padding: 3rem 0 3.5rem; } }
.eyebrow { font: 500 .78rem var(--mono); letter-spacing: .08em; text-transform: uppercase; color: #8fd0df; margin: 0 0 1rem; }
.hero h1 { font-size: clamp(2.2rem, 4.4vw, 3.5rem); line-height: 1.07; letter-spacing: -.03em; margin: 0 0 1.25rem; font-weight: 700; }
.hero p.lede { font-size: 1.2rem; color: #c6d4d3; max-width: 48rem; margin: 0 0 2rem; line-height: 1.65; }
.cta { display: flex; flex-wrap: wrap; gap: .75rem; }
.btn { display: inline-flex; align-items: center; gap: .5rem; padding: .7rem 1.15rem; border-radius: 6px;
  font-weight: 600; font-size: .95rem; text-decoration: none; transition: background-color .15s, transform .15s; }
.btn.primary { background: var(--teal); color: #fff; }
.btn.primary:hover { background: var(--teal-2); }
.btn.ghost { border: 1px solid #2d4a50; color: var(--cream); }
.btn.ghost:hover { background: var(--ink-2); }
.btn:active { transform: translateY(1px); }

.quickstart { margin-top: 1.75rem; background: var(--ink-2); border: 1px solid #1d3a40; border-radius: 8px; overflow: hidden; max-width: 48rem; }
.quickstart-head { display: flex; align-items: center; justify-content: space-between; padding: .5rem .85rem; background: rgba(0,0,0,0.25); border-bottom: 1px solid #1d3a40; font: 500 .74rem var(--mono); color: #8fd0df; text-transform: uppercase; letter-spacing: .06em; }
.quickstart-head span.sub { font-size: .72rem; color: #9fb3b1; text-transform: none; letter-spacing: normal; }
.quickstart pre { margin: 0; padding: .85rem 1rem; background: transparent; font: .8rem/1.65 var(--mono); color: #d5e2e0; border: none; }
.com { color: #6f8a88; }
.mark { justify-self: center; width: min(100%, 28rem); }
.mark img { width: 100%; height: auto; border-radius: 22px; display: block; }

.stats { border-top: 1px solid #1d3a40; display: grid; grid-template-columns: repeat(5, 1fr); }
.stats div { padding: 1.4rem 0; }
.stats b { display: block; font: 650 1.7rem var(--mono); font-variant-numeric: tabular-nums; color: var(--cream); letter-spacing: -.02em; }
.stats span { font-size: .82rem; color: #9fb3b1; }
@media (max-width: 720px) { .stats { grid-template-columns: repeat(2, 1fr); } }

section.band { padding: 5rem 0; border-bottom: 1px solid var(--line); }
.kicker { font: 500 .75rem var(--mono); letter-spacing: .08em; text-transform: uppercase; color: var(--teal); margin: 0 0 .6rem; }
@media (prefers-color-scheme: dark) { .kicker { color: #6cc3d8; } }
h2 { font-size: clamp(1.6rem, 3vw, 2.2rem); letter-spacing: -.025em; line-height: 1.15; margin: 0 0 1rem; }
.intro { color: var(--dim); max-width: 58rem; font-size: 1.05rem; margin: 0 0 2.5rem; }

.figure { background: var(--card); border: 1px solid var(--line); border-radius: 8px; padding: 1rem; overflow-x: auto; }
.arch { display: block; width: 100%; min-width: 680px; height: auto; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, sans-serif; }
.arch .dot { fill: var(--line); }
.arch .device rect:first-child, .arch .node > rect:first-child { fill: var(--bg); stroke: var(--line); }
.arch .glyph { fill: none; stroke: var(--dim); stroke-width: 1.5; }
.arch .core { fill: var(--ink); }
.arch .chips rect { fill: var(--card); stroke: var(--line); }
.arch .chips text { font: 13px var(--mono); fill: var(--fg); }
.arch .gate rect { fill: var(--teal); }
.arch .gate text { fill: #fff; }
.arch .h { font-size: 14px; font-weight: 600; fill: var(--fg); }
.arch .s { font-size: 12px; fill: var(--dim); }
.arch .inv { fill: var(--cream); }
.arch .c { text-anchor: middle; }
.arch .wire { fill: none; stroke: var(--teal); stroke-width: 1.6; }
.arch .dashed { stroke-dasharray: 5 4; }
.arch .tipfill { fill: var(--teal); }
.arch .pulse circle { fill: var(--teal-2); }
@media (prefers-reduced-motion: reduce) { .arch .pulse { display: none; } }

.steps { display: grid; grid-template-columns: repeat(auto-fit, minmax(15rem, 1fr)); gap: 1px; background: var(--line);
  border: 1px solid var(--line); border-radius: 8px; overflow: hidden; margin-top: 2rem; }
.steps div { background: var(--card); padding: 1.25rem 1.3rem; }
.steps b { font: 600 .8rem var(--mono); color: var(--teal); }
@media (prefers-color-scheme: dark) { .steps b { color: #6cc3d8; } }
.steps p { margin: .4rem 0 0; font-size: .93rem; color: var(--dim); }
.steps strong { color: var(--fg); }

.classes { display: grid; grid-template-columns: repeat(4, 1fr); gap: .75rem; margin-bottom: 1.5rem; }
@media (max-width: 800px) { .classes { grid-template-columns: repeat(2, 1fr); } }
.cls { border: 1px solid var(--line); border-radius: 6px; padding: 1rem 1.1rem 1.1rem; background: var(--card); position: relative; }
.cls::before { content: ""; position: absolute; left: 0; top: 0; bottom: 0; width: 4px; border-radius: 6px 0 0 6px; background: var(--tone); }
.cls code { font: 600 .78rem var(--mono); color: var(--dim); }
.cls h3 { margin: .2rem 0 .35rem; font-size: 1.05rem; }
.cls p { margin: 0; font-size: .88rem; color: var(--dim); }
.rule { display: flex; gap: .6rem; align-items: baseline; font-size: .95rem; margin: .35rem 0; }
.rule::before { content: "→"; color: var(--teal); font-family: var(--mono); }

.playground { display: grid; grid-template-columns: 1fr auto 1fr; gap: 1.25rem; align-items: center; margin-top: 2rem; }
@media (max-width: 800px) { .playground { grid-template-columns: 1fr; } .play-arrow { transform: rotate(90deg); margin: .5rem auto; } }
.play-pane { border: 1px solid var(--line); border-radius: 8px; background: var(--card); overflow: hidden; }
.play-header { display: flex; justify-content: space-between; align-items: center; padding: .6rem 1rem; border-bottom: 1px solid var(--line); font: 600 .82rem var(--mono); background: var(--bg); }
.play-pane pre { margin: 0; padding: 1rem; background: var(--ink); color: #d5e2e0; font: .78rem/1.55 var(--mono); border: none; }
.play-arrow { display: flex; flex-direction: column; align-items: center; gap: .35rem; text-align: center; font: 500 .75rem var(--mono); color: var(--teal); }
.play-arrow code { font-size: .72rem; color: var(--dim); background: var(--card); border: 1px solid var(--line); padding: .15rem .45rem; border-radius: 4px; }
.pill { display: inline-flex; align-items: center; gap: .3rem; font: 500 .7rem var(--mono); padding: .12rem .45rem; border-radius: 999px; }
.pill.proc { background: rgba(42, 138, 91, 0.15); color: var(--method-post); }
.pill.data { background: rgba(178, 115, 24, 0.15); color: var(--method-put); }

.split { display: grid; grid-template-columns: 1fr 1fr; gap: 1.5rem; }
@media (max-width: 860px) { .split { grid-template-columns: 1fr; } }
.panel { border: 1px solid var(--line); border-radius: 8px; background: var(--card); padding: 1.5rem; }
.panel h3 { margin: 0 0 .5rem; font-size: 1.1rem; }
.panel p { margin: 0 0 1rem; color: var(--dim); font-size: .95rem; }
.term { background: var(--ink); color: #d5e2e0; border-radius: 6px; font: .8rem/1.6 var(--mono); padding: 1rem 1.1rem; overflow-x: auto; margin: 0; position: relative; border: 1px solid rgba(255,255,255,0.06); }
.term .p { color: #6cc3d8; }
.term .k { color: #9fb3b1; }
.term .v { color: #f1d9a6; }
pre .copy-btn { position: absolute; top: .6rem; right: .6rem; background: rgba(255,255,255,0.08); border: 1px solid rgba(255,255,255,0.15); color: #a0b2b0; border-radius: 4px; padding: .25rem .45rem; cursor: pointer; transition: all .15s; font: 500 .72rem var(--mono); line-height: 1; }
pre .copy-btn:hover { background: rgba(255,255,255,0.18); color: #fff; }
pre .copy-btn.copied { background: #2a8a5b; color: #fff; border-color: #2a8a5b; }

.glass { display: grid; grid-template-columns: repeat(3, 1fr); gap: .5rem; }
.glass div { border: 1px solid var(--line); border-radius: 6px; padding: .7rem .8rem; font-size: .85rem; background: var(--bg); }
.glass b { display: block; font-size: .78rem; font-family: var(--mono); color: var(--dim); font-weight: 500; }

.areas { display: grid; grid-template-columns: repeat(auto-fill, minmax(19rem, 1fr)); gap: 1rem; }
.area { border: 1px solid var(--line); border-radius: 8px; background: var(--card); padding: 1.2rem 1.3rem; }
.area.wide { grid-column: 1 / -1; }
.area h3 { margin: 0; font-size: 1rem; display: flex; justify-content: space-between; }
.area .n { font: .8rem var(--mono); color: var(--dim); font-variant-numeric: tabular-nums; }
.area p { margin: .3rem 0 .8rem; font-size: .88rem; color: var(--dim); }
.area ul { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: .35rem; }
.area a { display: inline-block; font: .78rem var(--mono); padding: .2rem .5rem; border: 1px solid var(--line);
  border-radius: 4px; text-decoration: none; transition: border-color .15s, color .15s; }
.area a:hover { border-color: var(--teal); color: var(--teal); }

table { width: 100%; min-width: 40rem; border-collapse: collapse; font-size: .92rem; }
th, td { text-align: left; padding: .75rem .9rem; border-bottom: 1px solid var(--line); vertical-align: top; }
thead th { font: 500 .72rem var(--mono); text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
td.num { font-family: var(--mono); font-variant-numeric: tabular-nums; white-space: nowrap; }
td code { font: .78rem var(--mono); color: var(--dim); overflow-wrap: anywhere; }
.note { font-size: .85rem; color: var(--dim); margin-top: 1rem; max-width: 50rem; }

.docs { display: grid; grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr)); gap: 1rem; }
.docs a { display: block; border: 1px solid var(--line); border-radius: 8px; padding: 1.2rem; background: var(--card);
  text-decoration: none; transition: border-color .15s; }
.docs a:hover { border-color: var(--teal); }
.docs b { display: block; margin-bottom: .25rem; }
.docs span { font-size: .88rem; color: var(--dim); }
footer { padding: 2.5rem 0 3rem; font-size: .85rem; color: var(--dim); display: flex; flex-wrap: wrap; gap: 1rem 2rem; }

`;

export function renderLanding(model: SelfModel): string {
  const count = (kind: string) => model.units.filter((u) => u.kind === kind).length;
  const caps = count("capability");
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sjel — local-first data kernel and native GPUI client</title>
<meta name="description" content="Sjel stores personal life facts as deterministic, typed SQLite data on hardware you own. Sub-10ms native GPUI interface across macOS and iOS with mathematical data-class sandboxing.">
<link rel="icon" href="favicon.svg" type="image/svg+xml">
<style>${CSS}</style>
</head>
<body>
<div class="top">
  <div class="wrap">
    <nav class="bar" aria-label="Site">
      <a class="brand" href="./"><img src="favicon.svg" alt="" width="28" height="28">Sjel</a>
      <div class="links">
        <a class="opt" href="#how">How it works</a>
        <a class="opt" href="#privacy">Privacy</a>
        <a class="opt" href="#capabilities">Capabilities</a>
        <a href="docs/index.html">Docs</a>
        <a href="reference/index.html">Reference</a>
        <a href="roadmap/index.html">Roadmap</a>
        <a href="${REPO}" class="icon-link" aria-label="GitHub repository" title="GitHub">${GITHUB_ICON}</a>
      </div>
    </nav>
    <div class="hero">
      <div>
        <p class="eyebrow">Local-first kernel · Rust · macOS &amp; iOS</p>
        <h1>A local-first data kernel and native GPUI client.</h1>
        <p class="lede">Sjel stores personal life facts as deterministic, typed SQLite data on hardware you own. Cryptographically signed peering over Tailscale, sub-10ms native rendering across macOS and iOS, and mathematical data-class sandboxing for local and cloud models.</p>
        <div class="cta">
          <a class="btn primary" href="docs/index.html">
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"></path><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"></path></svg>
            Read the docs
          </a>
          <a class="btn ghost" href="${REPO}">
            ${GITHUB_ICON}
            <span>GitHub</span>
          </a>
        </div>
        <div class="quickstart">
          <div class="quickstart-head">
            <span>Evaluate in 2 minutes</span>
            <span class="sub">No personal credentials needed</span>
          </div>
          <pre class="term"><span class="p">$</span> git clone https://github.com/larsboes/Sjel.git &amp;&amp; cd Sjel
<span class="p">$</span> tools/demo-up    <span class="com"># run seeded demo stack &amp; mock services</span>
<span class="p">$</span> tools/doctor     <span class="com"># verify machine toolchain &amp; local health</span></pre>
        </div>
      </div>
      <div class="mark"><img src="hedgehog.png" alt="The Sjel hedgehog" width="680" height="461"></div>
    </div>
    <div class="stats" aria-label="Sjel in numbers">
      <div><b>${caps}</b><span>capabilities</span></div>
      <div><b>${count("lib")}</b><span>shared libraries</span></div>
      <div><b>${count("pack")}</b><span>agent skill packs</span></div>
      <div><b>${model.upstreams.length}</b><span>upstreams with a verdict</span></div>
      <div><b>1</b><span>native app, two platforms</span></div>
    </div>
  </div>
</div>

<main>
  <section class="band" id="how">
    <div class="wrap">
      <p class="kicker">How it works</p>
      <h2>One node holds the data. Every device talks to it.</h2>
      <p class="intro">A capability is a small local service with its own store and HTTP routes. The node runs the ones a household enables. The apps read and write through it, and model calls leave it only through a gate.</p>
      <div class="figure">${ARCHITECTURE}</div>
      <div class="steps">
        <div><b>01 · Deterministic Kernel</b><p><strong>One writer.</strong> Authoritative state lives in local SQLite stores on hardware you control. Peer devices sync via signed cryptographic keys over Tailscale or LAN.</p></div>
        <div><b>02 · Type-Level Privacy</b><p><strong>Four data classes.</strong> Facts about others (<code>c2</code>) and secrets (<code>c3</code>) never reach cloud models. Cloud calls receive strictly pseudonymized derivatives (<code>c1</code>).</p></div>
        <div><b>03 · Zero-Web-Lag Native UI</b><p><strong>Pure GPUI in Rust.</strong> Sub-10ms first-frame response across macOS and iOS. Zero Electron memory footprint, offline-first execution, and native Metal composition.</p></div>
        <div><b>04 · Grounded Agent Seat</b><p><strong>Sandboxed workflows.</strong> Assistant workflows run with bounded repository context, explicit command policies, and one-tap user approvals for write actions.</p></div>
      </div>
    </div>
  </section>

  <section class="band" id="privacy">
    <div class="wrap">
      <p class="kicker">Privacy</p>
      <h2>Every value has a data class.</h2>
      <p class="intro">The class decides where a value may go. One function decides it: <code>content_item::cloud_admission</code>, in a leaf crate every capability depends on.</p>
      <div class="classes">
        <div class="cls" style="--tone:#5f9a6e"><code>c0</code><h3>Public</h3><p>Already public. May reach a cloud model unchanged.</p></div>
        <div class="cls" style="--tone:#16748d"><code>c1</code><h3>Mine</h3><p>Your own facts. Reaches a cloud model only as a reviewed, redacted derivative.</p></div>
        <div class="cls" style="--tone:#b07d2a"><code>c2</code><h3>Others</h3><p>Facts about other people. Redacted before storage, never sent to a cloud model.</p></div>
        <div class="cls" style="--tone:#a8433a"><code>c3</code><h3>Secret</h3><p>Never reaches a cloud model, and never enters a local prompt either.</p></div>
      </div>
      <p class="rule">A class goes up by itself. Only a person, with a written reason, can lower it.</p>
      <p class="rule">A pseudonymizer sits in front of cloud model calls and admitted agent reads.</p>
      <p class="rule">Secrets live in the OS keychain. Everything else holds a reference to them.</p>

      <div class="playground">
        <div class="play-pane">
          <div class="play-header"><span>Household Fact</span><span class="pill data">c2 · Others</span></div>
          <pre class="term"><code>{
  "entity": "trip-booking-489",
  "traveler": "Elena Rostova",
  "email": "elena@example.com",
  "destination": "Köln Hbf",
  "fare_cents": 4290
}</code></pre>
        </div>
        <div class="play-arrow">
          <span>Class Gate</span>
          <code>content_item::cloud_admission</code>
          <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor"><path d="M5 12h14M12 5l7 7-7 7" stroke-width="2" stroke-linecap="round"/></svg>
        </div>
        <div class="play-pane">
          <div class="play-header"><span>Admitted Cloud Payload</span><span class="pill proc">c1 · Derivative</span></div>
          <pre class="term"><code>{
  "entity": "anon-person-7f2a",
  "traveler": "P-7f2a",
  "destination": "Köln Hbf",
  "fare_eur": 42.90,
  "_redacted": ["email"]
}</code></pre>
        </div>
      </div>
    </div>
  </section>

  <section class="band" id="surfaces">
    <div class="wrap">
      <p class="kicker">Two surfaces</p>
      <h2>Calm on the glass, open in the terminal.</h2>
      <div class="split">
        <div class="panel">
          <h3>On the glass</h3>
          <p>The native app is for daily life. A phone joins the Mac from a card that appears when both are on the same network. Nobody in the household needs a terminal.</p>
          <div class="glass">
            <div><b>Today</b>what waits for you</div>
            <div><b>Travel</b>trips and a native map</div>
            <div><b>People</b>who is around</div>
            <div><b>Calendar</b>reads and writes</div>
            <div><b>Inbox</b>capture anywhere</div>
            <div><b>Agent</b>sessions and approvals</div>
          </div>
        </div>
        <div class="panel">
          <h3>Under the hood</h3>
          <p>Every capability serves its own route manifest, speaks typed JSON over loopback HTTP and keeps its state in SQLite. Every workflow also runs from the terminal.</p>
<pre class="term"><span class="p">$</span> curl -s 127.0.0.1:8109/routes
{
  <span class="k">"capability"</span>: <span class="v">"web-links"</span>,
  <span class="k">"routes"</span>: [
    { <span class="k">"method"</span>: <span class="v">"GET"</span>, <span class="k">"path"</span>: <span class="v">"/api/links"</span>,
      <span class="k">"summary"</span>: <span class="v">"Saved web destinations, grouped by site…"</span> },
    { <span class="k">"method"</span>: <span class="v">"POST"</span>, <span class="k">"path"</span>: <span class="v">"/api/links"</span>,
      <span class="k">"request_schema"</span>: { … } }
  ]
}</pre>
        </div>
      </div>
    </div>
  </section>

  <section class="band" id="capabilities">
    <div class="wrap">
      <p class="kicker">Capabilities</p>
      <h2>${caps} capabilities, enabled per machine.</h2>
      <p class="intro">Public capabilities live in the repository. Private ones live in a deployment overlay. Each name opens its reference page: its contract, what it is coupled to and its routes.</p>
      <div class="areas">${capabilityIndex(model.units)}
      </div>
    </div>
  </section>

  <section class="band" id="measured">
    <div class="wrap">
      <p class="kicker">Measured</p>
      <h2>Each number names the command that reproduces it.</h2>
      <div class="figure">
        <table>
          <thead><tr><th>What</th><th>Result</th><th>Reproduce</th></tr></thead>
          <tbody>
            <tr><td>Pseudonymizer recall, reversible path</td><td class="num">48/48 · gate ≥ 91.7%</td><td><code>cargo run -p sjel-comms --bin comms-redaction-eval -- --pseudonymized &lt;corpus&gt;</code></td></tr>
            <tr><td>Redaction recall, destructive path</td><td class="num">48/48 · gate ≥ 91.7%</td><td><code>the same command without --pseudonymized</code></td></tr>
            <tr><td>Feed ranking (bge-m3)</td><td class="num">0.941 pairwise · 0.994 nDCG</td><td><code>bun capabilities/comms/eval/run-relevance.ts</code></td></tr>
            <tr><td>Mac app start and memory</td><td class="num">&lt; 10 ms · ~100 MB</td><td><code>SJEL_GPUI_FRAMES</code> and <code>ps -o rss=</code></td></tr>
          </tbody>
        </table>
      </div>
      <p class="note">The redaction corpus lives in private settings, so its two rows cannot be re-run from a public clone. Offline use and pairing time are not measured yet.</p>
    </div>
  </section>

  <section class="band" id="docs">
    <div class="wrap">
      <p class="kicker">Documentation</p>
      <h2>Go deeper.</h2>
      <div class="docs">
        <a href="docs/index.html"><b>Start here</b><span>Reading paths by task: install, place a component, diagnose a host.</span></a>
        <a href="docs/architecture/index.html"><b>Architecture</b><span>Component ownership, storage and access boundaries.</span></a>
        <a href="docs/architecture/security.html"><b>Security</b><span>Data classes, the agent gate and the threat model.</span></a>
        <a href="docs/capabilities/manifest.html"><b>Capability manifests</b><span>How a capability declares how it runs.</span></a>
        <a href="docs/operations/index.html"><b>Operations</b><span>Doctor, service lifecycle and patch policy.</span></a>
        <a href="reference/index.html"><b>Reference</b><span>One page per unit: contract, routes, coupling and README.</span></a>
        <a href="reference/self-model.html"><b>Self-model</b><span>Every unit, its coupling and every upstream verdict.</span></a>
        <a href="roadmap/index.html"><b>Roadmap</b><span>Timeline and milestones: Active, Planned, Launched.</span></a>
        <a href="${REPO}/blob/main/PLAN.md"><b>Plan</b><span>What is built, what is next and why.</span></a>
      </div>
    </div>
  </section>
</main>

<div class="wrap">
  <footer>
    <div class="foot-wrap">
      <span>Sjel · AGPL-3.0</span>
      <div class="foot-links">
        <a href="docs/index.html">Docs</a>
        <a href="reference/index.html">Reference</a>
        <a href="${REPO}/blob/main/CONTRIBUTING.md">Contributing</a>
        <a href="${REPO}" class="icon-link" aria-label="GitHub repository" title="GitHub">${GITHUB_ICON}</a>
      </div>
    </div>
  </footer>
</div>
</body>
</html>
`;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const outIdx = args.indexOf("--out");
  const outDir = outIdx >= 0 ? args[outIdx + 1] : join(SJEL_ROOT, "site");
  const model = JSON.parse(readFileSync(join(SJEL_ROOT, "self.json"), "utf8")) as SelfModel;
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, "index.html"), renderLanding(model));
  console.log(`landing: wrote ${join(outDir, "index.html")}`);
}
