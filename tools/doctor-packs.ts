// tools/doctor-packs.ts — the Pack sections of tools/doctor, as JSON for the Rust doctor.
//
// The doctor moved to Rust on 2026-10-02 (tools/sjel-cli/src/doctor/). Pack deployment state
// did not: it is the ledger and hashing logic of tools/harnesses.ts and tools/lib/pack-deploy.ts,
// about 2000 lines of TypeScript with its own port still ahead. So this sidecar runs the same
// rules doctor.ts ran and hands the lines over, and doctor prints them where they always stood.
// Run by doctor only; output is a JSON array of { name, lines: [{ level, message }] }.

import { join } from "node:path";
import { HARNESSES, harnessById, isInstalled } from "./lib/harness-registry.ts";
import { statusesFor } from "./harnesses.ts";

type Level = "ok" | "warn" | "bad";
type Section = { name: string; lines: Array<{ level: Level; message: string }> };

const home = process.env.HOME ?? "";
const hints: Record<string, { deploy: (pack: string) => string; sync: (pack: string) => string }> = {
  claude: { deploy: (pack) => `tools/packs.sh link ${pack}`, sync: (pack) => `tools/packs-claude sync ${pack}` },
  codex: { deploy: (pack) => `tools/packs-codex deploy ${pack}`, sync: (pack) => `tools/packs-codex sync ${pack}` },
  opencode: { deploy: (pack) => `tools/packs-opencode deploy ${pack}`, sync: (pack) => `tools/packs-opencode sync ${pack}` },
  pi: { deploy: (pack) => `tools/packs-pi deploy ${pack}`, sync: (pack) => `tools/packs-pi sync ${pack}` },
};

const sections: Section[] = [];

// One section per INSTALLED harness: which harnesses are here is the registry's answer, never a
// stale list (the 2026-09-07 scar: three Packs sat for an uninstalled Codex while the installed
// pi got zero rows).
for (const harness of HARNESSES) {
  if (!isInstalled(harness)) continue;
  const lines: Section["lines"] = [];
  const ok = (message: string) => lines.push({ level: "ok", message });
  const warn = (message: string) => lines.push({ level: "warn", message });
  const bad = (message: string) => lines.push({ level: "bad", message });
  try {
    const rows = statusesFor(harness);
    if (rows.length === 0) {
      warn("no Packs/*/pack.toml found");
    } else {
      const commands = hints[harness.id];
      const unselected: string[] = [];
      for (const row of rows) {
        const label = `${row.pack}/${row.skill}`;
        const detail = row.detail ? ` — ${row.detail}` : "";
        switch (row.status) {
          case "current":
            ok(`${label} current`);
            break;
          case "not-deployed":
            if (harness.model === "registry") unselected.push(row.pack);
            else warn(`${label} not deployed (${commands.deploy(row.pack)})`);
            break;
          case "discovered":
            ok(`${label} loaded by pi via discovery, not the ledger${detail}`);
            break;
          case "outdated":
            warn(`${label} outdated (${commands.sync(row.pack)})${detail}`);
            break;
          case "drifted":
            bad(`${label} has destination-side changes; sync/remove will refuse`);
            break;
          case "migration-required":
            warn(
              `${label} needs generated-artifact ledger migration${harness.id === "codex" ? ` (tools/packs-codex migrate-generated ${row.pack} --accept-current)` : ""}${detail}`,
            );
            break;
          case "missing":
            bad(`${label} is ledger-owned but missing${detail}`);
            break;
          case "collision":
            bad(
              `${label} destination is occupied by an unowned skill${harness.id === "claude" ? ` (tools/packs-claude adopt ${row.pack} if it is identical)` : ""}`,
            );
            break;
          case "invalid":
            bad(`${label} invalid${detail}`);
            break;
        }
      }
      if (harness.model === "registry" && unselected.length) {
        const packs = [...new Set(unselected)].sort();
        ok(`${packs.length} Pack(s) not selected for pi: ${packs.join(", ")} (selection is the design — profiles decide)`);
      }
    }
  } catch (error) {
    bad(`${harness.label} Pack state unreadable: ${(error as Error).message}`);
  }
  sections.push({ name: `Packs (${harness.label} deployed)`, lines });
}

// One warning per ABSENT harness that still holds deployed units.
for (const harness of HARNESSES) {
  if (isInstalled(harness) || harness.model !== "materialized") continue;
  const deployed = statusesFor(harness).filter((row) => row.status !== "not-deployed");
  if (!deployed.length) continue;
  const packs = [...new Set(deployed.map((row) => row.pack))].sort();
  const piReads = isInstalled(harnessById("pi")) && harness.config().destination === join(home, ".agents", "skills");
  const base = `${deployed.length} units from ${packs.length} Pack(s) sit at ${harness.config().destination} for a ${harness.label} that is not installed`;
  const message = piReads
    ? `${base} — pi IS installed and discovers that directory, so pi is loading these right now. ` +
      `Keep them in pi (tools/packs-pi deploy ${packs.join(" ")}) before removing; otherwise they leave both harnesses.`
    : `${base}; nothing reads them. Remove: ${harness.cli} remove ${packs.join(" ")}`;
  sections.push({ name: `Packs (${harness.label} NOT installed)`, lines: [{ level: "warn", message }] });
}

console.log(JSON.stringify(sections));
