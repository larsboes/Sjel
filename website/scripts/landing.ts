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
import { esc, GITHUB_ICON, themeControl, THEME_INIT_SCRIPT, THEME_SCRIPT, TOKENS } from "./style.ts";

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
    <rect x="24" y="80" width="196" height="110" rx="4"/>
    <rect x="42" y="100" width="30" height="56" rx="4" class="glyph"/><rect x="51" y="105" width="12" height="2" rx="1" class="glyph"/>
    <text x="88" y="118" class="h">iPhone app</text>
    <text x="88" y="140" class="s">GPUI · local copy</text>
    <text x="88" y="158" class="s">signs every request</text>
    <text x="88" y="176" class="s">Wi-Fi or Tailscale</text>
  </g>
  <g class="device">
    <rect x="24" y="270" width="196" height="110" rx="4"/>
    <rect x="44" y="294" width="40" height="28" rx="2" class="glyph"/><rect x="38" y="325" width="52" height="5" rx="1" class="glyph"/>
    <text x="104" y="308" class="h">Mac app</text>
    <text x="104" y="330" class="s">GPUI · native</text>
    <text x="104" y="348" class="s">loopback HTTP</text>
    <text x="104" y="366" class="s">first frame &lt; 10 ms</text>
  </g>

  <path id="p-phone" d="M220 135 C 262 135, 262 190, 304 190" class="wire dashed" marker-end="url(#tip)"/>
  <path id="p-mac" d="M220 325 C 262 325, 262 270, 304 270" class="wire" marker-end="url(#tip)"/>
  <path d="M220 135 C 262 135, 262 190, 304 190" class="wire-pulse wire-pulse-phone" aria-hidden="true"/>
  <path d="M220 325 C 262 325, 262 270, 304 270" class="wire-pulse wire-pulse-mac" aria-hidden="true"/>

  <g class="node">
    <rect x="304" y="24" width="316" height="412" rx="4"/>
    <text x="324" y="52" class="h">Node</text>
    <text x="324" y="70" class="s">a Mac or a Linux machine · holds every write</text>
    <rect x="324" y="86" width="276" height="74" rx="3" class="core"/>
    <text x="340" y="110" class="h">Core</text>
    <text x="340" y="130" class="s">entities · devices · data classes</text>
    <text x="340" y="148" class="s">pseudonymizer · SQLite stores</text>
    <g class="chips">
      <rect x="324" y="176" width="132" height="34" rx="2"/><text x="338" y="198">people</text>
      <rect x="468" y="176" width="132" height="34" rx="2"/><text x="482" y="198">travel</text>
      <rect x="324" y="220" width="132" height="34" rx="2"/><text x="338" y="242">money</text>
      <rect x="468" y="220" width="132" height="34" rx="2"/><text x="482" y="242">home</text>
      <rect x="324" y="264" width="132" height="34" rx="2"/><text x="338" y="286">calendar</text>
      <rect x="468" y="264" width="132" height="34" rx="2"/><text x="482" y="286">mail · feed</text>
      <rect x="324" y="308" width="132" height="34" rx="2"/><text x="338" y="330">documents</text>
      <rect x="468" y="308" width="132" height="34" rx="2"/><text x="482" y="330">agent</text>
    </g>
    <text x="324" y="374" class="s">Each capability: its own store and routes.</text>
    <text x="324" y="394" class="s">Private values live in the overlay.</text>
    <text x="324" y="414" class="s">Secrets live in the OS keychain.</text>
  </g>

  <path id="p-model" d="M620 230 L 706 230" class="wire" marker-end="url(#tip)"/>
  <path d="M620 230 L 706 230" class="wire-pulse wire-pulse-model" aria-hidden="true"/>
  <g class="gate">
    <rect x="636" y="196" width="52" height="68" rx="3"/>
    <text x="662" y="226" class="s c inv">class</text><text x="662" y="242" class="s c inv">gate</text>
  </g>
  <g class="device">
    <rect x="706" y="146" width="176" height="168" rx="4"/>
    <text x="722" y="174" class="h">Models</text>
    <text x="722" y="198" class="s">1 · a model on the device</text>
    <text x="722" y="216" class="s">2 · a model on the Mac</text>
    <text x="722" y="234" class="s">3 · fixed rules</text>
    <text x="722" y="264" class="s">Cloud gets c0, or c1</text>
    <text x="722" y="280" class="s">as a redacted derivative.</text>
    <text x="722" y="296" class="s">Never c2 or c3.</text>
  </g>
</svg>`;

/** Plain-text ASCII / Unicode box-drawing topology diagram. */
const ASCII_ARCHITECTURE = `  ┌─────────────────┐
  │   iPhone app    │
  │  GPUI · local   │────┐
  │ signs requests  │    │
  └─────────────────┘    │
                         ▼ (Wi-Fi or Tailscale)
  ┌─────────────────┐  ┌─────────────────────────────────────────────────────────┐         ┌─────────────────┐
  │     Mac app     │  │ Node (authoritative writes)                             │         │     Models      │
  │  GPUI · native  │──┼▶ Core (entities · pseudonymizer · SQLite)               │         │ 1· on device    │
  │ loopback < 10ms │  │ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌────────────────┐  │         │ 2· on Mac       │
  └─────────────────┘  │ │ people  │ │ travel  │ │  money  │ │ calendar       │  │  gate   │ 3· fixed rules  │
                       │ ├─────────┤ ├─────────┤ ├─────────┤ ├────────────────┤  │  ┌───┐  │                 │
                       │ │  home   │ │  agent  │ │  inbox  │ │ documents      │  │──┼▶ │──┼▶ Cloud gets c0  │
                       │ └─────────┘ └─────────┘ └─────────┘ └────────────────┘  │  └───┘  │  or reviewed c1 │
                       │ Each capability: own SQLite store + loopback HTTP       │         │  never c2 or c3 │
                       │ Private values in overlay · Secrets in OS keychain      │         │                 │
                       └─────────────────────────────────────────────────────────┘         └─────────────────┘`;

const CSS = `${TOKENS}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg); font: 400 16px/1.6 var(--sans); -webkit-font-smoothing: antialiased; -moz-osx-font-smoothing: grayscale; }
.hero > *, .split > *, .areas > *, .classes > *, .steps > *, .docs > * { min-width: 0; }
.btn svg, .btn .icon { flex-shrink: 0; }

.hero { display: grid; grid-template-columns: 1.25fr 1fr; gap: clamp(2rem, 5vw, 4.5rem); align-items: center; padding: 5rem 0 5.5rem; }
@media (max-width: 900px) { .hero { grid-template-columns: 1fr; padding: 3rem 0 3.5rem; } }
.eyebrow { font: 500 .78rem var(--mono); letter-spacing: .08em; text-transform: uppercase; color: var(--teal); margin: 0 0 1rem; }
.hero h1 { font: 400 clamp(2.8rem, 5.4vw, 5rem)/1.02 var(--display); font-style: italic; letter-spacing: -.035em; margin: 0 0 1.4rem; color: var(--fg); }
.hero p.lede { font-size: 1.15rem; color: var(--dim); max-width: 58ch; margin: 0 0 2rem; line-height: 1.65; }
.cta { display: flex; flex-wrap: wrap; gap: .75rem; }
.btn { display: inline-flex; align-items: center; gap: .5rem; padding: .7rem 1.15rem; border-radius: 2px;
  font-weight: 500; font-size: .95rem; text-decoration: none; transition: background-color .16s ease, border-color .16s ease, color .16s ease, box-shadow .16s ease; }
.btn.primary { background: #16748d; color: #ffffff; }
:root[data-theme="dark"] .btn.primary { background: var(--teal); color: #070c0d; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) .btn.primary { background: var(--teal); color: #070c0d; } }
.btn.primary:hover { opacity: .92; box-shadow: 0 4px 16px var(--teal-glow); }
.btn.ghost { border: 1px solid var(--line); color: var(--fg); background: transparent; }
.btn.ghost:hover { background: var(--teal-tint); border-color: var(--teal); color: var(--teal); }

.quickstart { margin: 1.75rem 0; background: var(--card); border: 1px solid var(--line); border-radius: 2px; overflow: hidden; max-width: 48rem; box-shadow: 0 4px 20px rgba(0,0,0,0.04); }
.quickstart-head { display: flex; align-items: center; justify-content: space-between; padding: .5rem .85rem; background: var(--code); border-bottom: 1px solid var(--line); font: 500 .74rem var(--mono); color: var(--fg); text-transform: uppercase; letter-spacing: .06em; }
.quickstart-head span.sub { font-size: .72rem; color: var(--dim); text-transform: none; letter-spacing: normal; }
.quickstart pre { margin: 0; padding: .85rem 1rem; background: var(--term-bg); font: .8rem/1.65 var(--mono); color: var(--term-fg); border: none; overflow-x: auto; }
.quickstart { scroll-margin-top: 5rem; }
.quickstart-head { flex-wrap: wrap; gap: .4rem; }
.quickstart-note { margin: 0; padding: .65rem 1rem; font-size: .82rem; color: var(--dim); }
.quickstart a { color: var(--teal); text-underline-offset: .15em; }
.quickstart pre .p { color: var(--teal); font-weight: 600; }
.com { color: var(--dim-subtle); }
.mark { justify-self: center; width: min(100%, 22rem); }
.mark img { width: 100%; height: auto; border-radius: 4px; border: 1px solid rgba(22, 116, 141, 0.25); display: block; box-shadow: 0 8px 30px rgba(22, 116, 141, 0.16); }

.stats { border-top: 1px solid var(--line); display: grid; grid-template-columns: repeat(4, 1fr); gap: 1.5rem; }
.stats div { padding: 1.4rem 0; }
.stats b { display: block; font: 400 1.1rem var(--display); color: var(--fg); letter-spacing: 0; }
.stats span { font-size: .82rem; color: var(--dim); }
@media (max-width: 820px) { .stats { grid-template-columns: repeat(2, 1fr); gap: .75rem; } }
@media (max-width: 500px) { .stats { grid-template-columns: 1fr; gap: 0; } }

/* Connected intelligence & Cross-domain joins */
.joins-grid { display: grid; grid-template-columns: 1.15fr 1fr; gap: 1.75rem; align-items: stretch; margin-top: 2rem; }
@media (max-width: 900px) { .joins-grid { grid-template-columns: 1fr; } }

.join-graph-card {
  background: var(--card); border: 1px solid var(--line); border-radius: 2px; padding: 1.5rem;
  display: flex; flex-direction: column; gap: 1rem; position: relative;
}
.join-hub {
  background: var(--code); border: 1px solid var(--teal); border-radius: 2px; padding: 1.1rem 1.25rem;
  box-shadow: 0 4px 16px var(--teal-tint);
}
.join-hub-badge {
  display: inline-flex; align-items: center; gap: .35rem; font: 600 .7rem var(--mono);
  color: var(--teal); background: var(--teal-tint); padding: .15rem .45rem; border-radius: 2px;
  text-transform: uppercase; letter-spacing: .06em; margin-bottom: .4rem;
}
.join-hub h3 { margin: 0 0 .25rem; font-size: 1.15rem; color: var(--fg); font-weight: 500; }
.join-hub p { margin: 0; font-size: .88rem; color: var(--dim); }

.join-nodes { display: grid; grid-template-columns: 1fr 1fr; gap: .75rem; }
@media (max-width: 600px) { .join-nodes { grid-template-columns: 1fr; } }
.join-node {
  background: var(--bg); border: 1px solid var(--line); border-radius: 2px; padding: .85rem 1rem;
  display: flex; flex-direction: column; gap: .25rem; transition: border-color .15s, transform .15s;
}
.join-node:hover { border-color: var(--teal); transform: translateY(-1px); }
.join-node-kind { font: 600 .68rem var(--mono); color: var(--teal); text-transform: uppercase; letter-spacing: .06em; }
.join-node-title { font-weight: 500; font-size: .92rem; color: var(--fg); }
.join-node-desc { font-size: .8rem; color: var(--dim); line-height: 1.45; }
.join-node-via { font: .72rem var(--mono); color: var(--dim-subtle); margin-top: .15rem; }

.join-value-card {
  display: flex; flex-direction: column; justify-content: space-between; gap: 1.25rem;
  background: var(--card); border: 1px solid var(--line); border-radius: 2px; padding: 1.5rem;
}
.join-value-block h4 { margin: 0 0 .35rem; font-size: 1rem; font-weight: 500; color: var(--fg); display: flex; align-items: center; gap: .5rem; }
.join-value-block h4 .icon-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--teal); display: inline-block; flex-shrink: 0; }
.join-value-block p { margin: 0; font-size: .88rem; color: var(--dim); line-height: 1.55; }
.join-query-box {
  background: var(--term-bg); border: 1px solid var(--line); border-radius: 2px; padding: .9rem 1.1rem;
  font: .78rem/1.55 var(--mono); color: var(--term-fg); margin-top: auto;
}
.join-query-box .q-label { color: var(--teal); font-weight: 600; text-transform: uppercase; font-size: .7rem; letter-spacing: .06em; display: block; margin-bottom: .3rem; }
.join-query-box .q-text { color: var(--term-fg); }
.join-query-box .q-ans { color: var(--dim-subtle); margin-top: .45rem; padding-top: .45rem; border-top: 1px dashed var(--line); display: block; }

.joins-toolbar {
  display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: .75rem;
  margin-top: 1.5rem; margin-bottom: .5rem;
}
.joins-tabs-label { font: 500 .72rem var(--mono); text-transform: uppercase; letter-spacing: .08em; color: var(--dim); }
.joins-tabs { display: inline-flex; gap: 3px; background: var(--code); border: 1px solid var(--line); border-radius: 3px; padding: 2px; }
.join-tab-btn {
  font: 500 .74rem var(--mono); padding: .25rem .65rem; border: none; background: transparent;
  color: var(--dim); border-radius: 2px; cursor: pointer; transition: color .15s, background-color .15s;
}
.join-tab-btn:hover { color: var(--fg); }
.join-tab-btn.active, .join-tab-btn[aria-pressed="true"] { background: var(--card); color: var(--teal); border: 1px solid var(--line); box-shadow: 0 1px 3px rgba(0,0,0,0.05); }
.join-node { cursor: pointer; user-select: none; }
.join-node.active { border-color: var(--teal); background: var(--card-hover); box-shadow: 0 0 0 1px var(--teal); }

section.band { padding: clamp(3.5rem, 8vw, 6.5rem) 0; border-bottom: 1px solid var(--line); }
.kicker { font: 500 .75rem var(--mono); letter-spacing: .08em; text-transform: uppercase; color: var(--teal); margin: 0 0 .6rem; }
h2 { font: 400 clamp(1.6rem, 3vw, 2.2rem)/1.15 var(--display); letter-spacing: -.025em; margin: 0 0 1rem; color: var(--fg); }
.intro { color: var(--dim); max-width: 58rem; font-size: 1.05rem; margin: 0 0 2.5rem; }

.arch-frame { background: var(--card); border: 1px solid var(--line); border-radius: 2px; overflow: hidden; margin-top: .5rem; }
.arch-toggle-bar { display: flex; align-items: center; justify-content: space-between; padding: .55rem 1rem; border-bottom: 1px solid var(--line); background: var(--code); font: 500 .74rem var(--mono); text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
.arch-toggle-btns { display: inline-flex; gap: 2px; background: var(--card); border: 1px solid var(--line); border-radius: 2px; padding: 2px; }
.arch-btn { font: 500 .72rem var(--mono); padding: .2rem .55rem; border: none; background: transparent; color: var(--dim); border-radius: 2px; cursor: pointer; transition: color .15s, background-color .15s; }
.arch-btn:hover { color: var(--teal); }
.arch-btn[aria-pressed="true"] { background: var(--term-bg); color: var(--term-fg); box-shadow: inset 0 0 0 1px var(--teal); }
.arch-btn:focus-visible { outline: 2px solid var(--focus); outline-offset: 1px; }
.arch-canvas { padding: 1rem; overflow-x: auto; -webkit-overflow-scrolling: touch; }
.arch-canvas[hidden] { display: none; }
.arch-pre { margin: 0; font: .8rem/1.38 var(--mono); color: var(--fg); border: none; background: transparent; padding: .6rem; }
.arch-fallback { display: none; }
@media (scripting: none) {
  .arch-toggle-btns { display: none; }
  .arch-fallback { display: block; border-top: 1px solid var(--line); }
  .arch-fallback summary { padding: .6rem 1rem; font: 500 .78rem var(--mono); color: var(--dim); cursor: pointer; }
  .arch-fallback summary:hover { color: var(--fg); }
}

.figure { background: var(--card); border: 1px solid var(--line); border-radius: 2px; padding: 1rem; overflow-x: auto; -webkit-overflow-scrolling: touch; }
.arch { display: block; width: 100%; min-width: 680px; height: auto; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, sans-serif; }
.arch .dot { fill: var(--line); }
.arch .device rect:first-child, .arch .node > rect:first-child { fill: var(--card); stroke: var(--line); }
.arch .glyph { fill: none; stroke: var(--dim); stroke-width: 1.5; }
.arch .core { fill: var(--bg); stroke: var(--line); }
.arch .chips rect { fill: var(--bg); stroke: var(--line); }
.arch .chips text { font: 13px var(--mono); fill: var(--fg); }
.arch .gate rect { fill: #16748d; stroke: #16748d; }
:root[data-theme="dark"] .arch .gate rect { fill: #16748d; stroke: var(--teal); stroke-width: 1.5; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) .arch .gate rect { fill: #16748d; stroke: var(--teal); stroke-width: 1.5; } }
.arch .gate text.inv { fill: #ffffff; font-weight: 600; }
.arch .h { font-size: 14px; font-weight: 500; fill: var(--fg); }
.arch .s { font-size: 12px; fill: var(--dim); }
.arch .c { text-anchor: middle; }
.arch .wire { fill: none; stroke: var(--teal); stroke-width: 1.6; }
.arch .dashed { stroke-dasharray: 4 4; }
.arch .tipfill { fill: var(--teal); }
.arch .wire-pulse {
  fill: none;
  stroke: var(--teal-bright);
  stroke-width: 2.2px;
  stroke-linecap: round;
  stroke-dasharray: 14 180;
  stroke-dashoffset: 194;
  animation: wire-flow 2.8s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  opacity: 0.95;
  filter: drop-shadow(0 0 3px var(--teal));
}
.arch .wire-pulse-phone { animation-delay: 0s; }
.arch .wire-pulse-mac { animation-delay: 0.95s; }
.arch .wire-pulse-model { animation-delay: 1.85s; stroke-dasharray: 10 90; stroke-dashoffset: 100; }

@keyframes wire-flow {
  0% { stroke-dashoffset: 194; opacity: 0; }
  12% { opacity: 1; }
  85% { opacity: 1; }
  100% { stroke-dashoffset: 0; opacity: 0; }
}

.steps { display: grid; grid-template-columns: repeat(auto-fit, minmax(15rem, 1fr)); gap: 0; background: transparent;
  border-top: 1px solid var(--line); margin-top: 2rem; }
.steps div { background: transparent; border-bottom: 1px solid var(--line); padding: 1.25rem 1.3rem; }
.steps b { font: 500 .8rem var(--mono); color: var(--teal); }
.steps p { margin: .4rem 0 0; font-size: .93rem; color: var(--dim); }
.steps strong { color: var(--fg); font-weight: 500; }

.classes { display: grid; grid-template-columns: repeat(4, 1fr); gap: .75rem; margin-bottom: 1.5rem; }
@media (max-width: 800px) { .classes { grid-template-columns: repeat(2, 1fr); } }
.cls { border: 1px solid var(--line); border-top: 3px solid var(--tone, var(--line)); border-radius: 2px; padding: 1.1rem; background: var(--card); transition: border-color .16s ease, transform .16s ease; position: relative; }
.cls:hover { border-color: var(--tone, var(--teal)); transform: translateY(-1px); }
.cls .cls-tick { position: absolute; top: 6px; right: 8px; font: 400 10px var(--mono); color: var(--dim-subtle); opacity: .4; user-select: none; transition: opacity .15s, color .15s; }
.cls:hover .cls-tick { opacity: 1; color: var(--tone, var(--teal)); }
.cls code { display: inline-block; font: 600 .75rem var(--mono); color: var(--fg); background: var(--code); padding: .12rem .42rem; border-radius: 2px; margin-bottom: .4rem; }
.cls h3 { margin: 0 0 .35rem; font-size: 1.05rem; font-weight: 500; color: var(--fg); }
.cls p { margin: 0; font-size: .88rem; color: var(--dim); }
.rule { display: flex; gap: .6rem; align-items: baseline; font-size: .95rem; margin: .35rem 0; color: var(--fg); }
.rule::before { content: "→"; color: var(--teal); font-family: var(--mono); font-weight: 600; }

.gate-ctrl-bar {
  display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: .85rem;
  background: var(--code); border: 1px solid var(--line); border-bottom: none; border-radius: 2px 2px 0 0;
  padding: .65rem 1rem; margin-top: 1.75rem;
}
.gate-ctrl-group { display: flex; align-items: center; gap: .5rem; flex-wrap: wrap; }
.gate-label { font: 600 .72rem var(--mono); text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
.gate-btn-group { display: inline-flex; gap: 2px; background: var(--card); border: 1px solid var(--line); border-radius: 2px; padding: 2px; }
.gate-opt-btn {
  font: 500 .72rem var(--mono); padding: .22rem .55rem; border: none; background: transparent;
  color: var(--dim); border-radius: 2px; cursor: pointer; transition: all .15s ease;
}
.gate-opt-btn:hover { color: var(--fg); }
.gate-opt-btn.active { background: var(--teal); color: #ffffff; }
.gate-opt-btn.cls-btn-c0.active { background: #2a8a5b; }
.gate-opt-btn.cls-btn-c1.active { background: #16748d; }
.gate-opt-btn.cls-btn-c2.active { background: #b27318; }
.gate-opt-btn.cls-btn-c3.active { background: #b73e34; }
.play-frame { border-radius: 2px; overflow: hidden; margin-top: 1.75rem; }
.play-frame .gate-ctrl-bar { margin-top: 0; }
.play-frame .playground { margin-top: 0; }

.playground { display: grid; grid-template-columns: 1fr auto 1fr; gap: 1.25rem; align-items: center; }
@media (max-width: 800px) { .playground { grid-template-columns: 1fr; } .play-arrow { transform: rotate(90deg); margin: .5rem auto; } }
.play-pane { border: 1px solid var(--line); border-radius: 2px; background: var(--card); overflow: hidden; transition: border-color .2s ease, box-shadow .2s ease; }
.play-pane.refused { border-color: #b73e34; }
.play-pane.refused .play-header { background: rgba(183, 62, 52, 0.08); }
.play-pane.admitted { border-color: var(--teal); }
.play-header { display: flex; justify-content: space-between; align-items: center; padding: .6rem 1rem; border-bottom: 1px solid var(--line); font: 500 .82rem var(--mono); background: var(--code); color: var(--fg); }
.play-pane pre { margin: 0; padding: 1rem; background: var(--term-bg); color: var(--term-fg); font: .78rem/1.55 var(--mono); border: none; min-height: 5.5rem; }
.play-arrow { display: flex; flex-direction: column; align-items: center; gap: .35rem; text-align: center; font: 500 .75rem var(--mono); color: var(--teal); transition: color .2s ease; }
.play-arrow.refused { color: #b73e34; }
.play-arrow.admitted { color: var(--teal); }
.play-arrow code { font-size: .72rem; color: var(--dim); background: var(--card); border: 1px solid var(--line); padding: .15rem .45rem; border-radius: 2px; }

.split { display: grid; grid-template-columns: 1fr 1fr; gap: 1.5rem; }
@media (max-width: 860px) { .split { grid-template-columns: 1fr; } }
.panel { border: 1px solid var(--line); border-radius: 2px; background: var(--card); padding: 1.5rem; }
.panel h3 { margin: 0 0 .5rem; font-size: 1.1rem; font-weight: 500; color: var(--fg); }
.panel p { margin: 0 0 1rem; color: var(--dim); font-size: .95rem; }
.term { background: var(--term-bg); color: var(--term-fg); border-radius: 2px; font: .8rem/1.6 var(--mono); padding: 1rem 1.1rem; overflow-x: auto; margin: 0; position: relative; border: 1px solid var(--line); }
.term .p { color: var(--teal-bright); font-weight: 500; }
.term .k { color: var(--term-fg); font-weight: 500; }
.term .v { color: var(--term-dim); }

.glass { display: grid; grid-template-columns: repeat(3, 1fr); gap: .5rem; }
.glass div { border: 1px solid var(--line); border-radius: 2px; padding: .7rem .8rem; font-size: .85rem; background: var(--bg); color: var(--dim); }
.glass b { display: block; font-size: .78rem; font-family: var(--mono); color: var(--fg); font-weight: 500; }

.areas { display: grid; grid-template-columns: repeat(auto-fill, minmax(19rem, 1fr)); gap: 1rem; }
.area { border: 1px solid var(--line); border-radius: 2px; background: var(--card); padding: 1.2rem 1.3rem; transition: border-color .16s ease; }
.area:hover { border-color: var(--teal); }
.area.wide { grid-column: 1 / -1; }
.area h3 { margin: 0; font-size: 1rem; font-weight: 500; color: var(--fg); display: flex; justify-content: space-between; align-items: center; }
.area .n { font: 500 .78rem var(--mono); color: var(--teal); background: var(--teal-tint); padding: .1rem .4rem; border-radius: 2px; font-variant-numeric: tabular-nums; }
.area p { margin: .3rem 0 .8rem; font-size: .88rem; color: var(--dim); }
.area ul { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: .35rem; }
.area a { display: inline-block; font: .78rem var(--mono); padding: .2rem .5rem; border: 1px solid var(--line);
  border-radius: 2px; text-decoration: none; color: var(--dim); transition: border-color .15s, color .15s, background-color .15s; }
.area a:hover { border-color: var(--teal); color: var(--teal); background: var(--card-hover); }

table { width: 100%; min-width: 40rem; border-collapse: collapse; font-size: .92rem; }
th, td { text-align: left; padding: .75rem .9rem; border-bottom: 1px solid var(--line); vertical-align: top; color: var(--fg); }
thead th { font: 500 .72rem var(--mono); text-transform: uppercase; letter-spacing: .06em; color: var(--dim); }
td.num { font-family: var(--mono); font-variant-numeric: tabular-nums; white-space: nowrap; }
td code { font: .78rem var(--mono); color: var(--dim); overflow-wrap: anywhere; }
.note { font-size: .85rem; color: var(--dim); margin-top: 1rem; max-width: 50rem; }

.docs { display: grid; grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr)); gap: 1rem; }
.docs a { display: block; border: 1px solid var(--line); border-radius: 2px; padding: 1.2rem; background: var(--card);
  text-decoration: none; color: inherit; transition: border-color .16s ease, background-color .16s ease, transform .16s ease; }
.docs a:hover { border-color: var(--teal); background: var(--card-hover); transform: translateY(-1px); }
.docs b { display: block; margin-bottom: .25rem; font-weight: 500; color: var(--fg); transition: color .15s; }
.docs a:hover b { color: var(--teal); }
.docs span { font-size: .88rem; color: var(--dim); }

footer { padding: 2.5rem 0 3rem; font-size: .85rem; color: var(--dim); border-top: 1px solid var(--line); }
.foot-wrap { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 1rem 2rem; width: 100%; }
.foot-links { display: flex; align-items: center; gap: 1.25rem; flex-wrap: wrap; }
.foot-links a { color: var(--dim); text-decoration: none; transition: color .15s; }
.foot-links a:hover { color: var(--teal); }

@keyframes reveal { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }
.hero > div:first-child { animation: reveal .36s ease-out both; }
.hero .mark { animation: reveal .36s .06s ease-out both; }

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: .01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: .01ms !important;
    scroll-behavior: auto !important;
  }
}
`;

export function renderLanding(model: SelfModel): string {
  const count = (kind: string) => model.units.filter((u) => u.kind === kind).length;
  const caps = count("capability");
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Sjel — personal data on hardware you control</title>
<meta name="description" content="Sjel keeps personal records in SQLite on hardware you control, with a native Mac app, an iPhone client in development, and a data-class gate for hosted model calls.">
<link rel="icon" href="favicon.svg" type="image/svg+xml">
${THEME_SCRIPT}
<style>${CSS}</style>
</head>
<body>
<div class="top">
  <div class="wrap">
    <nav class="bar" aria-label="Site">
      <a class="brand" href="./"><img src="favicon.svg" alt="" width="28" height="28">Sjel</a>
      <div class="links">
        <a class="opt" href="#how">How it works</a>
        <a class="opt" href="#joins">Connections</a>
        <a class="opt" href="#privacy">Privacy</a>
        <a class="opt" href="#capabilities">Capabilities</a>
        <a href="docs/index.html">Docs</a>
        <a href="reference/index.html">Reference</a>
        <a href="roadmap/index.html">Roadmap</a>
        <a href="${REPO}" class="icon-link" aria-label="GitHub repository" title="GitHub">${GITHUB_ICON}</a>
        ${themeControl()}
      </div>
    </nav>
    <div class="hero">
      <div>
        <p class="eyebrow">Local-first kernel · Rust · macOS &amp; iOS</p>
        <h1>Your life data, at home.</h1>
        <div class="quickstart blueprint" id="install" role="region" aria-labelledby="install-title">
          <div class="quickstart-head">
            <span id="install-title">Install the CLI</span>
            <span class="sub">macOS ARM64 · Linux x86-64/ARM64</span>
          </div>
          <p class="quickstart-note"><strong>Awaiting public release.</strong> This URL is not live yet.</p>
          <pre class="term"><code>curl -LsSf ${REPO}/releases/latest/download/install.sh | sh</code></pre>
          <p class="quickstart-note">CLI and coding resources only. No source checkout or Rust toolchain.
            <a href="docs/operations/index.html#cli-only-coding-installation">wget, pinned versions and setup</a> ·
            <a href="docs/index.html">Documentation</a>
          </p>
        </div>
        <p class="lede">Sjel connects the fragmented areas of your life—travel, calendar, finance, contacts, notes, and documents—into one private SQLite system on hardware you control. A data-class gate keeps combined analysis safe at home, so models only see what you explicitly admit.</p>
        <div class="cta">
          <a class="btn primary" href="#install">Install the CLI</a>
          <a class="btn ghost" href="${REPO}">
            ${GITHUB_ICON}
            <span>GitHub</span>
          </a>
        </div>
      </div>
      <div class="mark"><img src="hedgehog.svg" alt="The Sjel hedgehog" width="600" height="407"></div>
    </div>
    <div class="stats" aria-label="How Sjel is built">
      <div><b>Connected</b><span>Typed joins across 45 capabilities</span></div>
      <div><b>Local-first</b><span>SQLite on hardware you control</span></div>
      <div><b>Native</b><span>Mac app; iPhone client in development</span></div>
      <div><b>Private</b><span>Four data classes &amp; local-only models</span></div>
    </div>
  </div>
</div>

<main>
  <section class="band" id="how">
    <div class="wrap">
      <p class="kicker">How it works</p>
      <h2>One node holds the data. Every device talks to it.</h2>
      <p class="intro">A capability is a small local service with its own store and HTTP routes. The node runs the ones a household enables. The apps read and write through it, and model calls leave it only through a gate.</p>
      <div class="arch-frame blueprint">
        <div class="arch-toggle-bar">
          <span>System topology</span>
          <div class="arch-toggle-btns" role="group" aria-label="Diagram format">
            <button type="button" class="arch-btn" aria-pressed="true" aria-controls="arch-vector" data-arch="vector">Vector</button>
            <button type="button" class="arch-btn" aria-pressed="false" aria-controls="arch-ascii" data-arch="ascii">ASCII</button>
          </div>
        </div>
        <div class="arch-canvas arch-vector" id="arch-vector" role="region" aria-label="Vector architecture diagram">
          ${ARCHITECTURE}
        </div>
        <div class="arch-canvas arch-ascii" id="arch-ascii" role="region" aria-label="ASCII architecture diagram" hidden>
          <pre class="term arch-pre"><code>${ASCII_ARCHITECTURE}</code></pre>
        </div>
        <details class="arch-fallback">
          <summary>View plain-text ASCII diagram</summary>
          <pre class="term arch-pre"><code>${ASCII_ARCHITECTURE}</code></pre>
        </details>
      </div>
      <div class="steps">
        <div><b>01 · Deterministic Kernel</b><p><strong>One writer.</strong> Authoritative state lives in local SQLite stores on hardware you control. Peer devices sync via signed cryptographic keys over Tailscale or LAN.</p></div>
        <div><b>02 · Data-class gate</b><p><strong>Four classes.</strong> Hosted models may receive public data, or a reviewed c1 derivative. Data about other people and secrets have no cloud representation.</p></div>
        <div><b>03 · Native app</b><p><strong>Mac and iPhone.</strong> The client is built with Rust and GPUI. Its interfaces use the same local service contracts as the command line.</p></div>
        <div><b>04 · Bounded agents</b><p><strong>Access follows policy.</strong> Agent sessions use declared access tiers and command rules. Write actions require approval when the active policy asks for it.</p></div>
      </div>
    </div>
  </section>

  <section class="band" id="joins">
    <div class="wrap">
      <p class="kicker">Connected intelligence</p>
      <h2>Single-purpose tools hold fragments. Sjel connects the dots.</h2>
      <p class="intro">Most software isolates your life into disconnected silos: your bank knows nothing about your trips; your calendar doesn't know who lives in the city you visit; your note app has no link to your expenses. Sjel is intentionally scaled across 45 capabilities so every record can cross-reference the rest. Through typed cross-capability joins (<code>sjel-links</code>), disparate data sources unite in one local graph to deliver combined analysis impossible in fragmented tools. The scenarios below use synthetic records; they are not live Sjel data or demonstrated query results.</p>
      <div class="joins-toolbar">
        <span class="joins-tabs-label">Explore joined scenarios</span>
        <div class="joins-tabs" role="group" aria-label="Joined data scenarios">
          <button type="button" class="join-tab-btn active" aria-pressed="true" data-scenario="trip">Trip &amp; Transit</button>
          <button type="button" class="join-tab-btn" aria-pressed="false" data-scenario="health">Doctor &amp; Health</button>
          <button type="button" class="join-tab-btn" aria-pressed="false" data-scenario="lease">Lease &amp; Relocation</button>
        </div>
      </div>
      <div class="joins-grid blueprint">
        <div class="join-graph-card">
          <div class="join-hub">
            <span class="join-hub-badge" id="join-hub-badge">Hub Record · trip:plan:8f4c21</span>
            <h3 id="join-hub-title">Trip to Zürich &amp; ETH Workshop</h3>
            <p id="join-hub-desc">5 cross-capability joins resolved in local SQLite</p>
          </div>
          <div class="join-nodes" id="join-nodes">
            <div class="join-node active" data-node-idx="0" tabindex="0" role="button" aria-pressed="true">
              <span class="join-node-kind">cal:entry · Calendar</span>
              <span class="join-node-title">ETH Research Colloquium</span>
              <span class="join-node-desc">Thursday 14:00 · Auditorium HG E 7</span>
              <span class="join-node-via">via event materialization</span>
            </div>
            <div class="join-node" data-node-idx="1" tabindex="0" role="button" aria-pressed="false">
              <span class="join-node-kind">transit · Travel</span>
              <span class="join-node-title">ICE 71 · Sparpreis Booking</span>
              <span class="join-node-desc">Seat 42 · Carriage 9 · Punctuality live</span>
              <span class="join-node-via">via trip_id ledger</span>
            </div>
            <div class="join-node" data-node-idx="2" tabindex="0" role="button" aria-pressed="false">
              <span class="join-node-kind">fin:tx · Finance</span>
              <span class="join-node-title">€89.90 Bahn + CHF 45 Dinner</span>
              <span class="join-node-desc">2 ledger transactions tied to trip</span>
              <span class="join-node-via">via source_id link</span>
            </div>
            <div class="join-node" data-node-idx="3" tabindex="0" role="button" aria-pressed="false">
              <span class="join-node-kind">ent &amp; place · Network</span>
              <span class="join-node-title">Marcus (Collaborator)</span>
              <span class="join-node-desc">Resident of Zürich · Contact card</span>
              <span class="join-node-via">via place:stable_id</span>
            </div>
          </div>
        </div>
        <div class="join-value-card">
          <div class="join-value-block">
            <h4><span class="icon-dot" aria-hidden="true"></span>Combined query value</h4>
            <p>Ask cross-domain questions no siloed application can answer: <em>"What did my Zürich trip actually cost across transit, food, and stays?"</em> or <em>"Who in my network lives near my next travel destination?"</em></p>
          </div>
          <div class="join-value-block">
            <h4><span class="icon-dot" aria-hidden="true"></span>Safe local synthesis</h4>
            <p>Because every dot connects in local SQLite on your Mac, local models and agents can synthesize across your entire life history—without shipping your financial ledger, schedule, or contacts to cloud data aggregators.</p>
          </div>
          <div class="join-value-block">
            <h4><span class="icon-dot" aria-hidden="true"></span>Zero vendor lock-in</h4>
            <p>Obsidian stays Obsidian, Apple Calendar stays Apple Calendar. Sjel reads and links them through typed stable IDs (<code>kind:rest</code>) without trapping your data inside a proprietary walled garden.</p>
          </div>
          <div class="join-query-box">
            <span class="q-label">Illustrative query · not an executed command or result</span>
            <span class="q-text" id="join-q-text">A future local query could ask: “What did the Zürich trip cost, and who did I meet?”</span>
            <span class="q-ans" id="join-q-ans">Illustrative only. Sjel does not currently provide this query command or verify these results.</span>
          </div>
        </div>
      </div>
    </div>
  </section>

  <section class="band" id="privacy">
    <div class="wrap">
      <p class="kicker">Privacy</p>
      <h2>Every value has a data class.</h2>
      <p class="intro">The cloud gate checks the original class, its derivative and the declared tier. The shared function is <code>content_item::cloud_admission</code>, in a leaf crate used by every capability.</p>
      <div class="classes blueprint">
        <div class="cls" style="--tone:#2a8a5b;--tone-bg:rgba(42,138,91,0.09)"><span class="cls-tick" aria-hidden="true">+</span><code>c0</code><h3>Public</h3><p>Already public. May reach a cloud model unchanged.</p></div>
        <div class="cls" style="--tone:#16748d;--tone-bg:rgba(22,116,141,0.09)"><span class="cls-tick" aria-hidden="true">+</span><code>c1</code><h3>Mine</h3><p>Your own facts. Reaches a cloud model only as a reviewed, redacted derivative.</p></div>
        <div class="cls" style="--tone:#b27318;--tone-bg:rgba(178,115,24,0.09)"><span class="cls-tick" aria-hidden="true">+</span><code>c2</code><h3>Others</h3><p>Facts about other people. Redacted before storage, never sent to a cloud model.</p></div>
        <div class="cls" style="--tone:#b73e34;--tone-bg:rgba(183,62,52,0.09)"><span class="cls-tick" aria-hidden="true">+</span><code>c3</code><h3>Secret</h3><p>Never reaches a cloud model, and never enters a local prompt either.</p></div>
      </div>
      <p class="rule">A class goes up by itself. Only a person, with a written reason, can lower it.</p>
      <p class="rule">A pseudonymizer sits in front of cloud model calls and admitted agent reads.</p>
      <p class="rule">Secrets live in the OS keychain. Everything else holds a reference to them.</p>

      <div class="play-frame blueprint">
        <div class="gate-ctrl-bar">
          <div class="gate-ctrl-group">
            <span class="gate-label">Data class</span>
            <div class="gate-btn-group" role="group" aria-label="Select data class">
              <button type="button" class="gate-opt-btn cls-btn-c0" aria-pressed="false" data-gate-cls="c0">c0 Public</button>
              <button type="button" class="gate-opt-btn cls-btn-c1" aria-pressed="false" data-gate-cls="c1">c1 Mine</button>
              <button type="button" class="gate-opt-btn cls-btn-c2 active" aria-pressed="true" data-gate-cls="c2">c2 Others</button>
              <button type="button" class="gate-opt-btn cls-btn-c3" aria-pressed="false" data-gate-cls="c3">c3 Secret</button>
            </div>
          </div>
          <div class="gate-ctrl-group">
            <span class="gate-label">Destination</span>
            <div class="gate-btn-group" role="group" aria-label="Select destination model">
              <button type="button" class="gate-opt-btn" aria-pressed="false" data-gate-target="local">Local Mac Model</button>
              <button type="button" class="gate-opt-btn active" aria-pressed="true" data-gate-target="cloud">Cloud Model API</button>
            </div>
          </div>
        </div>
        <div class="playground">
          <div class="play-pane" id="gate-in-pane">
            <div class="play-header"><span>Input record</span><span class="pill data" id="gate-in-pill">c2 · Others</span></div>
            <pre class="term"><code id="gate-in-code">{
  "class": "c2",
  "content": "not shown"
}</code></pre>
          </div>
          <div class="play-arrow" id="gate-arrow">
            <span>Class Gate</span>
            <code>content_item::cloud_admission</code>
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor"><path d="M5 12h14M12 5l7 7-7 7" stroke-width="2" stroke-linecap="round"/></svg>
          </div>
          <div class="play-pane refused" id="gate-out-pane">
            <div class="play-header"><span>Gate result</span><span class="pill data" id="gate-out-pill">Refused</span></div>
            <pre class="term"><code id="gate-out-code">{
  "admitted": false,
  "reason": "c2 has no cloud representation"
}</code></pre>
          </div>
        </div>
      </div>
    </div>
  </section>

  <section class="band" id="surfaces">
    <div class="wrap">
      <p class="kicker">Two surfaces</p>
      <h2>One system, two ways in.</h2>
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
        <a href="roadmap/index.html">Roadmap</a>
        <a href="${REPO}/blob/main/CONTRIBUTING.md">Contributing</a>
        <a href="${REPO}" class="icon-link" aria-label="GitHub repository" title="GitHub">${GITHUB_ICON}</a>
      </div>
    </div>
  </footer>
</div>
<script>
(function() {
  // Architecture diagram switcher
  var btns = document.querySelectorAll(".arch-btn");
  var vectorView = document.querySelector(".arch-vector");
  var asciiView = document.querySelector(".arch-ascii");
  if (btns.length && vectorView && asciiView) {
    for (var i = 0; i < btns.length; i++) {
      (function(b) {
        b.addEventListener("click", function() {
          var mode = b.getAttribute("data-arch");
          var showAscii = mode === "ascii";
          for (var j = 0; j < btns.length; j++) {
            var active = btns[j] === b;
            btns[j].setAttribute("aria-pressed", active ? "true" : "false");
          }
          vectorView.hidden = showAscii;
          asciiView.hidden = !showAscii;
        });
      })(btns[i]);
    }
  }

  // Connected intelligence scenario switcher & node inspector
  var SCENARIOS = {
    trip: {
      badge: "Hub Record · trip:plan:8f4c21",
      title: "Trip to Zürich & ETH Workshop",
      desc: "5 cross-capability joins resolved in local SQLite",
      qText: "A future local query could ask: “What did the Zürich trip cost, and who did I meet?”",
      qAns: "Illustrative only. Sjel does not currently provide this query command or verify these results.",
      nodes: [
        { kind: "cal:entry · Calendar", title: "ETH Research Colloquium", desc: "Thursday 14:00 · Auditorium HG E 7", via: "via event materialization" },
        { kind: "transit · Travel", title: "ICE 71 · Sparpreis Booking", desc: "Seat 42 · Carriage 9 · Punctuality live", via: "via trip_id ledger" },
        { kind: "fin:tx · Finance", title: "€89.90 Bahn + CHF 45 Dinner", desc: "2 ledger transactions tied to trip", via: "via source_id link" },
        { kind: "ent & place · Network", title: "Marcus (Collaborator)", desc: "Resident of Zürich · Contact card", via: "via place:stable_id" }
      ]
    },
    health: {
      badge: "Hub Record · health:visit:3d19b7",
      title: "Dr. Weiss Consultation & Bloodwork",
      desc: "4 cross-capability joins resolved in local SQLite",
      qText: "A future local query could ask: “Cross-reference my lab results with my medication log.”",
      qAns: "Illustrative only. Sjel does not currently provide this query command or verify these results.",
      nodes: [
        { kind: "cal:entry · Calendar", title: "Annual Health Checkup", desc: "Friday 09:30 · Praxis am Markt", via: "via appointment record" },
        { kind: "doc:store · Documents", title: "Lab Results PDF", desc: "Ferritin 48 · Vit-D 32 · Signed hash", via: "via SHA-256 attachment" },
        { kind: "fin:tx · Finance", title: "€42.50 Co-pay & Pharmacy", desc: "Insurance claim ledger #781", via: "via claim transaction" },
        { kind: "notes · Knowledge", title: "Medication Follow-up", desc: "Obsidian note Health/2026-Weiss.md", via: "via obsidian-vault link" }
      ]
    },
    lease: {
      badge: "Hub Record · proj:reloc:55a04e",
      title: "Apartment Handover & Address Registration",
      desc: "4 cross-capability joins resolved in local SQLite",
      qText: "A future local query could ask: “What checklist items remain for my lease and address registration?”",
      qAns: "Illustrative only. Sjel does not currently provide this query command or verify these results.",
      nodes: [
        { kind: "cal:entry · Calendar", title: "Key Handover Inspection", desc: "Monday 11:00 · Mainzer Str. 14", via: "via calendar inspection event" },
        { kind: "doc:store · Documents", title: "Signed Rental Contract PDF", desc: "PDF SHA-256 matched · Deposit receipt", via: "via document ledger" },
        { kind: "fin:tx · Finance", title: "€1,450 Security Deposit Wire", desc: "Bank transfer ledger link #902", via: "via escrow ledger link" },
        { kind: "place · Network", title: "Bürgeramt Registration Slot", desc: "Appointment confirmation & ID checklist", via: "via municipal place link" }
      ]
    }
  };

  var joinTabs = document.querySelectorAll(".join-tab-btn");
  var hubBadge = document.getElementById("join-hub-badge");
  var hubTitle = document.getElementById("join-hub-title");
  var hubDesc = document.getElementById("join-hub-desc");
  var qText = document.getElementById("join-q-text");
  var qAns = document.getElementById("join-q-ans");
  var joinNodes = document.querySelectorAll(".join-node");

  function renderScenario(key) {
    var data = SCENARIOS[key];
    if (!data) return;
    if (hubBadge) hubBadge.textContent = data.badge;
    if (hubTitle) hubTitle.textContent = data.title;
    if (hubDesc) hubDesc.textContent = data.desc;
    if (qText) qText.textContent = data.qText;
    if (qAns) qAns.textContent = data.qAns;
    for (var i = 0; i < joinNodes.length; i++) {
      var n = joinNodes[i];
      var nd = data.nodes[i];
      if (!nd) continue;
      var kindEl = n.querySelector(".join-node-kind");
      var titleEl = n.querySelector(".join-node-title");
      var descEl = n.querySelector(".join-node-desc");
      var viaEl = n.querySelector(".join-node-via");
      if (kindEl) kindEl.textContent = nd.kind;
      if (titleEl) titleEl.textContent = nd.title;
      if (descEl) descEl.textContent = nd.desc;
      if (viaEl) viaEl.textContent = nd.via;
      n.classList.toggle("active", i === 0);
      n.setAttribute("aria-pressed", i === 0 ? "true" : "false");
    }
  }

  for (var k = 0; k < joinTabs.length; k++) {
    (function(btn) {
      btn.addEventListener("click", function() {
        var scen = btn.getAttribute("data-scenario");
        for (var l = 0; l < joinTabs.length; l++) {
          var act = joinTabs[l] === btn;
          joinTabs[l].classList.toggle("active", act);
          joinTabs[l].setAttribute("aria-pressed", act ? "true" : "false");
        }
        renderScenario(scen);
      });
    })(joinTabs[k]);
  }

  for (var m = 0; m < joinNodes.length; m++) {
    (function(node) {
      node.addEventListener("click", function() {
        for (var n = 0; n < joinNodes.length; n++) {
          var sel = joinNodes[n] === node;
          joinNodes[n].classList.toggle("active", sel);
          joinNodes[n].setAttribute("aria-pressed", sel ? "true" : "false");
        }
      });
      node.addEventListener("keydown", function(e) {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          node.click();
        }
      });
    })(joinNodes[m]);
  }

  // Privacy Gate interactive test bench
  var GATE_EVAL = {
    "c0:cloud": {
      inPill: "c0 · Public", inPillClass: "pill-c0",
      inJson: '{\n  "class": "c0",\n  "content": "Public transit timetable schedule"\n}',
      status: "admitted",
      outPill: "Admitted", outPillClass: "pill-admitted",
      outJson: '{\n  "admitted": true,\n  "derivative_class": "c0",\n  "reason": "c0 is public data: reaches cloud unchanged"\n}'
    },
    "c1:cloud": {
      inPill: "c1 · Mine", inPillClass: "pill-c1",
      inJson: '{\n  "class": "c1",\n  "content": "Personal ticket: Passenger #482"\n}',
      status: "pseudonymized",
      outPill: "Pseudonymized", outPillClass: "pill-pseudonymized",
      outJson: '{\n  "admitted": true,\n  "derivative_class": "c1_pseudonymized",\n  "reason": "c1 admitted as reviewed pseudonymized derivative"\n}'
    },
    "c2:cloud": {
      inPill: "c2 · Others", inPillClass: "pill-c2",
      inJson: '{\n  "class": "c2",\n  "content": "not shown"\n}',
      status: "refused",
      outPill: "Refused", outPillClass: "pill-refused",
      outJson: '{\n  "admitted": false,\n  "reason": "c2 has no cloud representation"\n}'
    },
    "c3:cloud": {
      inPill: "c3 · Secret", inPillClass: "pill-c3",
      inJson: '{\n  "class": "c3",\n  "content": "<keychain ref: api_token_9f>"\n}',
      status: "blocked",
      outPill: "Blocked", outPillClass: "pill-blocked",
      outJson: '{\n  "admitted": false,\n  "reason": "c3 secret stays in OS keychain; never leaves device"\n}'
    },
    "c0:local": {
      inPill: "c0 · Public", inPillClass: "pill-c0",
      inJson: '{\n  "class": "c0",\n  "content": "Public transit timetable schedule"\n}',
      status: "admitted",
      outPill: "Local Model", outPillClass: "pill-admitted",
      outJson: '{\n  "admitted": true,\n  "target": "local_mac_model",\n  "reason": "Local inference on Mac hardware; 0 bytes sent over network"\n}'
    },
    "c1:local": {
      inPill: "c1 · Mine", inPillClass: "pill-c1",
      inJson: '{\n  "class": "c1",\n  "content": "Personal ticket: Passenger #482"\n}',
      status: "admitted",
      outPill: "Local Model", outPillClass: "pill-admitted",
      outJson: '{\n  "admitted": true,\n  "target": "local_mac_model",\n  "reason": "Local inference on Mac hardware; 0 bytes sent over network"\n}'
    },
    "c2:local": {
      inPill: "c2 · Others", inPillClass: "pill-c2",
      inJson: '{\n  "class": "c2",\n  "content": "Marcus contact card and notes"\n}',
      status: "admitted",
      outPill: "Local Model", outPillClass: "pill-admitted",
      outJson: '{\n  "admitted": true,\n  "target": "local_mac_model",\n  "reason": "Allowed for local prompt processing under user policy"\n}'
    },
    "c3:local": {
      inPill: "c3 · Secret", inPillClass: "pill-c3",
      inJson: '{\n  "class": "c3",\n  "content": "<keychain ref: api_token_9f>"\n}',
      status: "blocked",
      outPill: "Blocked", outPillClass: "pill-blocked",
      outJson: '{\n  "admitted": false,\n  "reason": "c3 secret never enters model prompts; reference only"\n}'
    }
  };

  var curCls = "c2";
  var curTarget = "cloud";
  var clsBtns = document.querySelectorAll("[data-gate-cls]");
  var targetBtns = document.querySelectorAll("[data-gate-target]");
  var inPill = document.getElementById("gate-in-pill");
  var inCode = document.getElementById("gate-in-code");
  var outPill = document.getElementById("gate-out-pill");
  var outCode = document.getElementById("gate-out-code");
  var arrow = document.getElementById("gate-arrow");
  var inPane = document.getElementById("gate-in-pane");
  var outPane = document.getElementById("gate-out-pane");

  function updateGate() {
    var key = curCls + ":" + curTarget;
    var res = GATE_EVAL[key];
    if (!res) return;
    if (inPill) {
      inPill.textContent = res.inPill;
      inPill.className = "pill data " + res.inPillClass;
    }
    if (inCode) inCode.textContent = res.inJson;
    if (outPill) {
      outPill.textContent = res.outPill;
      outPill.className = "pill data " + res.outPillClass;
    }
    if (outCode) outCode.textContent = res.outJson;
    if (arrow) {
      arrow.className = "play-arrow " + res.status;
    }
    if (outPane) {
      outPane.className = "play-pane " + res.status;
    }
  }

  for (var p = 0; p < clsBtns.length; p++) {
    (function(b) {
      b.addEventListener("click", function() {
        curCls = b.getAttribute("data-gate-cls");
        for (var q = 0; q < clsBtns.length; q++) {
          var act = clsBtns[q] === b;
          clsBtns[q].classList.toggle("active", act);
          clsBtns[q].setAttribute("aria-pressed", act ? "true" : "false");
        }
        updateGate();
      });
    })(clsBtns[p]);
  }

  for (var r = 0; r < targetBtns.length; r++) {
    (function(b) {
      b.addEventListener("click", function() {
        curTarget = b.getAttribute("data-gate-target");
        for (var s = 0; s < targetBtns.length; s++) {
          var act = targetBtns[s] === b;
          targetBtns[s].classList.toggle("active", act);
          targetBtns[s].setAttribute("aria-pressed", act ? "true" : "false");
        }
        updateGate();
      });
    })(targetBtns[r]);
  }
})();
</script>
${THEME_INIT_SCRIPT}
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
