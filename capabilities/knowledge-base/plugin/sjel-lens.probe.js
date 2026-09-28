"use strict";
/*
 * The falsifier: run Sjel Lens' adapters against the live loopback services and print what
 * they return for real notes.
 *
 * Not a test and not a gate. `sjel-lens.test.js` pins the pure functions against recorded
 * shapes; this answers the question a recorded shape cannot — whether those functions still
 * describe the services that are running right now. Run it after changing an adapter, and
 * whenever a capability's route changes under it.
 *
 *   bun capabilities/knowledge-base/plugin/sjel-lens.probe.js --vault "<vault root>"
 *
 * The vault root is a machine fact and is never written down here; the trips section needs
 * it to read a projection's frontmatter, and says so and skips if it is absent. The people
 * section needs no vault at all — vault's own answer carries the note paths.
 *
 * ## The transport is the point
 *
 * The stub `requestUrl` below is node's http with no Origin header, which is exactly what
 * Obsidian's `requestUrl` produces: the renderer hands the call to the main process over
 * the `request-url` IPC channel and the main process issues it with Electron's
 * `net.request`, setting only Content-Type and the caller's own headers. A `fetch` from
 * Obsidian's renderer would carry `Origin: app://obsidian.md`; the origin guard admits it,
 * but sjel-status sends no CORS header, so the renderer would withhold the registry. The
 * plugin's own header says why it uses `requestUrl` only.
 */

const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const path = require("node:path");
const { loadPlugin } = require("./load-plugin.js");

/* --------------------------------------------------------------- the transport */

/** Obsidian's requestUrl contract, over node. No Origin header, same as the real one. */
function requestUrl(options) {
  const url = new URL(options.url);
  const agent = url.protocol === "https:" ? https : http;
  return new Promise((resolve, reject) => {
    const request = agent.request(url, { method: options.method || "GET" }, (response) => {
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () => {
        const text = Buffer.concat(chunks).toString("utf8");
        let parsed = null;
        try {
          parsed = JSON.parse(text);
        } catch {
          parsed = null;
        }
        resolve({ status: response.statusCode, text, json: parsed, headers: response.headers });
      });
    });
    request.on("error", reject);
    request.end();
  });
}

const obsidian = {
  ItemView: class {},
  Notice: class {},
  Plugin: class {},
  PluginSettingTab: class {},
  Setting: class {},
  requestUrl,
  setIcon: () => {},
};

const { lens } = loadPlugin(path.join(__dirname, "sjel-lens"), obsidian).exports;

/* ------------------------------------------------------------------- arguments */

function vaultRoot() {
  const flag = process.argv.indexOf("--vault");
  if (flag !== -1 && process.argv[flag + 1]) return process.argv[flag + 1];
  return process.env.SJEL_LENS_VAULT || null;
}

/**
 * The frontmatter of one note, for the keys a projection writes.
 *
 * Inside Obsidian the plugin never parses YAML — it reads `metadataCache`. This stands in
 * for that one cache, and only handles the `key: "value"` lines the trips projection emits
 * (`capabilities/trips/src/projection.rs`). A note it cannot parse is a note this probe
 * cannot speak for, which is why it reports what it read.
 */
function frontmatterOf(file) {
  const text = fs.readFileSync(file, "utf8");
  if (!text.startsWith("---\n")) return null;
  const end = text.indexOf("\n---", 4);
  if (end === -1) return null;
  const matter = {};
  for (const line of text.slice(4, end).split("\n")) {
    const at = line.indexOf(":");
    if (at === -1) continue;
    const key = line.slice(0, at).trim();
    let value = line.slice(at + 1).trim();
    if (value.startsWith('"') && value.endsWith('"') && value.length > 1) value = value.slice(1, -1);
    matter[key] = value;
  }
  return matter;
}

/* ------------------------------------------------------------------------ probe */

const REGISTRY_URL = lens.DEFAULTS.registryUrl;
const TIMEOUT = lens.DEFAULTS.timeoutMs;

async function main() {
  let failures = 0;
  const say = (line) => process.stdout.write(`${line}\n`);

  say(`registry   ${REGISTRY_URL}`);
  let registry;
  try {
    registry = await lens.readJson(REGISTRY_URL, TIMEOUT);
  } catch (error) {
    say(`REFUSED    ${error.message}`);
    say("Nothing else can run: discovery is where every address comes from.");
    process.exit(1);
  }
  const serving = registry.filter((row) => lens.capabilityOrigin(row) !== null);
  say(`           ${registry.length} capabilities, ${serving.length} with an HTTP surface, ${serving.filter((r) => r.up).length} up`);
  say("");

  /* --- people ------------------------------------------------------------- */

  const vault = lens.resolveCapability(registry, "vault");
  say(`people     vault → ${vault.ok ? vault.origin : `REFUSED: ${vault.reason}`}`);
  if (vault.ok) {
    const payload = await lens.readJson(`${vault.origin}/api/people`, TIMEOUT);
    say(`           ${payload.people} people, ${payload.carrying_any} carrying a key, ${payload.disagreeing} disagreeing`);
    // One note that disagrees and one that does not: an instrument that answers the same
    // for both is measuring something other than agreement.
    const disagreeing = payload.facts.find((fact) => fact.disagrees.length > 0);
    const agreeing = payload.facts.find((fact) => fact.disagrees.length === 0 && Object.keys(fact.stored).length > 0);
    for (const fact of [disagreeing, agreeing]) {
      if (!fact) continue;
      const reading = lens.peopleReading(payload, fact.id);
      say(`           ${fact.id}`);
      for (const row of reading.rows) {
        const stored = row.stored === null ? "not stored" : row.stored;
        say(`             ${row.key.padEnd(14)} computed ${String(row.computed).padEnd(12)} note ${String(stored).padEnd(12)} ${row.disagrees ? "DISAGREES" : "agrees"}`);
      }
    }
    const missing = lens.peopleReading(payload, "Atlas/People/No Such Person.md");
    say(`           a note vault has no row for → found=${missing.found}`);
    // vault serves the path the filesystem gave it; Obsidian hands the plugin the composed
    // form. Ask for a decomposed row the way Obsidian would ask for it. No name is printed:
    // the count is the measurement, and this file is read on a public repository.
    const decomposed = payload.facts.filter((fact) => fact.id !== fact.id.normalize("NFC"));
    if (decomposed.length === 0) {
      say("           no NFD path under Atlas/People/ today, so the NFC comparison is untested here");
    } else {
      const found = decomposed.filter((fact) => lens.peopleReading(payload, fact.id.normalize("NFC")).found);
      say(`           ${decomposed.length} NFD path(s) asked for in NFC, as Obsidian hands them over → ${found.length} found`);
      if (found.length !== decomposed.length) failures += 1;
    }
  } else {
    failures += 1;
  }
  say("");

  /* --- trips -------------------------------------------------------------- */

  const trips = lens.resolveCapability(registry, "trips");
  say(`trips      trips → ${trips.ok ? trips.origin : `REFUSED: ${trips.reason}`}`);
  if (!trips.ok) {
    failures += 1;
  } else {
    const plans = await lens.readJson(`${trips.origin}/api/plans`, TIMEOUT);
    say(`           ${plans.length} plans`);
    const root = vaultRoot();
    if (!root) {
      say("           SKIPPED: pass --vault <root> to compare real projections");
      failures += 1;
    } else {
      const folder = path.join(root, "Resources", "Axon", "Trips");
      const notes = fs.readdirSync(folder).filter((name) => name.endsWith(".md")).sort();
      const counts = {};
      for (const name of notes) {
        const verdict = lens.tripStaleness(frontmatterOf(path.join(folder, name)), plans);
        counts[verdict.state] = (counts[verdict.state] || 0) + 1;
        const label = lens.stalenessLabel(verdict);
        say(`             ${label.badge.toUpperCase().padEnd(16)} ${name}`);
      }
      say(`           ${notes.length} notes: ${Object.entries(counts).map(([k, v]) => `${v} ${k}`).join(", ")}`);
      // Plant one: a projection one hour behind its plan must not read as current.
      const first = plans[0];
      if (first) {
        const planted = lens.tripStaleness(
          { axon_trip_id: first.id, axon_revision: String(Number(first.updated_at) - 3600) },
          plans
        );
        say(`           planted a revision 1 hour behind ${first.id} → ${planted.state}, ${lens.stalenessLabel(planted).detail}`);
        if (planted.state !== "stale") failures += 1;
      }
    }
  }
  say("");

  /* --- degrade ------------------------------------------------------------ */

  // The other half of the same instrument: a capability that is down must produce a line,
  // and a different one for each reason it cannot be reached.
  say("degrade");
  for (const name of ["punctuality", "knowledge-base", "no-such-capability"]) {
    const found = lens.resolveCapability(registry, name);
    say(`           ${name.padEnd(20)} ${found.ok ? `up at ${found.origin}` : `REFUSED: ${found.reason}`}`);
  }
  const dead = await lens
    .readJson("http://127.0.0.1:8085/health", 1000)
    .then(() => "answered — punctuality is up after all")
    .catch((error) => `REFUSED: ${error.message}`);
  say(`           a real request to a stopped capability → ${dead}`);
  say("");

  say(failures === 0 ? "probe: every adapter answered" : `probe: ${failures} section(s) could not answer`);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((error) => {
  process.stderr.write(`probe failed: ${error && error.stack ? error.stack : error}\n`);
  process.exit(1);
});
