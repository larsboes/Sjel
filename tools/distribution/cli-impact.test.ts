import { expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { cliImpact } from "./cli-impact.ts";

const root = resolve(import.meta.dir, "../..");

for (const [files, expected] of [
  [["crates/comms/clipper/package.json", "crates/comms/clipper/bun.lock"], "None of 2"],
  [["apps/gpui/ui/src/lib.rs"], "None of 1"],
  [["Packs/harness/pi-packages/pi-subagents/package.json"], "part of the CLI payload"],
  [["tools/lib/paths.sh"], "part of the CLI payload"],
  [["crates/pseudonymize/src/lib.rs"], "CLI build dependency"],
  [["crates/comms/Cargo.toml"], "compile-time input"],
  [["Cargo.lock"], "preview configuration"],
  [["docs/operations/index.md"], "None of 1"],
] as const) {
  test(`CLI preview selects ${files.join(", ")} only when it can affect the installer`, () => {
    const result = cliImpact(root, [...files], []);
    expect(result.reason).toContain(expected);
    expect(result.run).toBe(!expected.startsWith("None"));
  });
}

test("removed inputs and unavailable discovery keep the installer matrix", () => {
  expect(cliImpact(root, ["Packs/removed/pack.toml"], ["Packs/removed/pack.toml"]).run).toBe(true);
  const scratch = mkdtempSync(join(tmpdir(), "sjel-cli-impact-"));
  try {
    const result = cliImpact(scratch, ["unclassified.txt"], []);
    expect(result.run).toBe(true);
    expect(result.reason).toContain("CLI input discovery unavailable");
  } finally { rmSync(scratch, { recursive: true, force: true }); }
});
