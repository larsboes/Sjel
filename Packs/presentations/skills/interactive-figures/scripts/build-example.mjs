import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { leverState } from "../assets/lever-model.mjs";

const destination = new URL("../assets/lever.html", import.meta.url);
const modelSource = readFileSync(new URL("../assets/lever-model.mjs", import.meta.url), "utf8")
  .replace("export function leverState(", "function leverState(");

/** Build a standalone page from the same model used by the browser controls. */
export function exampleHtml() {
  const initial = leverState(2);
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:">
<title>Lever equilibrium</title>
<style>
:root { color-scheme: light dark; --paper: #f5f8f8; --ink: #203a43; --accent: #087f8c; --load: #a64c25; --line: #93a9ad; }
@media (prefers-color-scheme: dark) { :root { --paper: #15252c; --ink: #e3efef; --accent: #70d2dc; --load: #ffb18d; --line: #6c939d; } }
* { box-sizing: border-box; }
body { margin: 0; padding: 24px; background: var(--paper); color: var(--ink); font: 17px/1.5 system-ui, sans-serif; }
main { max-width: 760px; margin: auto; }
h1 { font-size: clamp(1.6rem, 5vw, 2.2rem); margin: 0 0 8px; }
p { max-width: 65ch; }
svg { display: block; width: 100%; height: auto; overflow: visible; }
svg text { fill: var(--ink); font: 14px system-ui, sans-serif; }
.controls { display: grid; grid-template-columns: 1fr auto; gap: 8px 16px; align-items: center; }
input { grid-column: 1 / -1; width: 100%; accent-color: var(--accent); }
input:focus-visible { outline: 3px solid var(--accent); outline-offset: 6px; }
.readout { font-size: 1.3rem; }
.assumptions { border-top: 1px solid var(--line); padding-top: 16px; }
@media (max-width: 420px) { body { padding: 16px; } }
@media print { input { display: none; } }
</style>
</head>
<body>
<main>
<h1>A longer arm needs less force</h1>
<p>Move the effort point while the load stays at 100 N, one metre from the fulcrum.</p>
<figure style="margin: 0">
<svg id="lever" viewBox="0 90 560 240" role="img" aria-labelledby="lever-title lever-description">
<title id="lever-title">An ideal lever in static balance</title>
<desc id="lever-description">The effort arm is 2 metres. A 50 newton effort balances a 100 newton load at one metre.</desc>
<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="9" markerHeight="9" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="context-stroke"/></marker></defs>
<path d="M 278 264 L 300 216 L 322 264 Z" fill="var(--line)" stroke="var(--ink)"/>
<rect id="beam" x="${initial.effortX - 18}" y="200" width="${initial.loadX - initial.effortX + 36}" height="16" rx="3" fill="var(--line)" stroke="var(--ink)"/>
<line id="effort-arrow" x1="${initial.effortX}" x2="${initial.effortX}" y1="${initial.effortArrowTop}" y2="199" stroke="var(--accent)" stroke-width="3" marker-end="url(#arrow)"/>
<line x1="350" x2="350" y1="${initial.loadArrowTop}" y2="199" stroke="var(--load)" stroke-width="3" marker-end="url(#arrow)"/>
<text id="effort-label" x="${initial.effortX}" y="${initial.effortArrowTop - 18}" text-anchor="middle">50.0 N effort</text>
<text x="358" y="${initial.loadArrowTop - 18}">100 N load</text>
<line id="effort-dimension" x1="${initial.effortX}" x2="300" y1="285" y2="285" stroke="var(--accent)"/>
<line x1="300" x2="350" y1="285" y2="285" stroke="var(--load)"/>
<text id="arm-label" x="250" y="308" text-anchor="middle">2.0 m</text>
<text x="332" y="308" text-anchor="middle">1 m</text>
<text x="300" y="258" text-anchor="middle" style="font-size:12px">pivot</text>
</svg>
<figcaption>Both sides produce the same turning moment: <output id="moment">${initial.moment}</output> N·m.</figcaption>
</figure>
<div class="controls">
<label for="arm">Effort arm</label><span><output id="arm-value">2.0</output> m</span>
<input id="arm" type="range" min="0.5" max="4" step="0.5" value="2" aria-describedby="force-text">
</div>
<p id="force-text" class="readout">Required effort: <output id="force" aria-live="polite">50.0</output> N</p>
<p class="assumptions">Model: a rigid, massless lever, a frictionless fulcrum and static equilibrium. Effort × effort arm = load × load arm. This is not a motion simulation.</p>
<noscript><p>JavaScript is unavailable. The figure shows the default balance: 50 N at 2 m.</p></noscript>
</main>
<script>
${modelSource}
const element = (id) => document.getElementById(id);
const arm = element("arm");
function paint() {
  const state = leverState(Number(arm.value));
  const force = state.effortForce.toFixed(1);
  element("arm-value").textContent = state.effortArm.toFixed(1);
  element("force").textContent = force;
  element("moment").textContent = String(state.moment);
  element("beam").setAttribute("x", state.effortX - 18);
  element("beam").setAttribute("width", state.loadX - state.effortX + 36);
  for (const attr of ["x1", "x2"]) element("effort-arrow").setAttribute(attr, state.effortX);
  element("effort-arrow").setAttribute("y1", state.effortArrowTop);
  element("effort-label").setAttribute("x", state.effortX);
  element("effort-label").setAttribute("y", state.effortArrowTop - 18);
  element("effort-label").textContent = force + " N effort";
  element("effort-dimension").setAttribute("x1", state.effortX);
  element("arm-label").setAttribute("x", (state.effortX + 300) / 2);
  element("arm-label").textContent = state.effortArm.toFixed(1) + " m";
  element("lever-description").textContent = "An effort of " + force + " N at " + state.effortArm + " m balances a 100 N load at 1 m.";
  element("lever").dataset.effortForce = String(state.effortForce);
  element("lever").dataset.effortArm = String(state.effortArm);
}
arm.addEventListener("input", paint);
paint();
</script>
</body>
</html>
`;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const args = process.argv.slice(2);
  if (args.length > 1 || (args.length === 1 && args[0] !== "--check")) {
    console.error("Usage: bun --no-env-file scripts/build-example.mjs [--check]");
    process.exitCode = 2;
  } else if (args[0] === "--check") {
    if (readFileSync(destination, "utf8") !== exampleHtml()) {
      console.error("Example is stale. Run bun --no-env-file scripts/build-example.mjs.");
      process.exitCode = 1;
    } else console.log("Lever example is current.");
  } else {
    writeFileSync(destination, exampleHtml());
    console.log("Built assets/lever.html.");
  }
}
