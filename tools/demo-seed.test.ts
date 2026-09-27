// Tests for tools/demo-seed.ts.
//
// One property, and it is a security one. `activeOverlay` asks paths.sh where the overlay is
// by running bash, and the answer decides whether demo-seed is allowed to write at all. It
// used to build that bash program by interpolating the checkout path into the `-c` string, so
// a checkout directory whose name contained `$(` or `"` was executed rather than read
// (CodeQL js/shell-command-injection-from-environment, tools/demo-seed.ts). The path is now a
// positional argument.
//
// The test runs the real function against a real hostile directory name rather than asserting
// something about the source text, because the shape of the argv is not the property -- the
// property is that the injected command does not run.

import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { activeOverlay } from "./demo-seed.ts";
import { VOCABULARY } from "./lib/demo-data.ts";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The second property, and it is a correctness one rather than a security one.
// tools/demo-up generates demo/overlay/config/finance.json with an `instruments`
// list, and the finance capability keys the holdings projection on the CANONICAL
// instrument names -- the reviewed import resolves the broker symbol to the
// canonical through instrument_aliases. The two lists live in two languages and
// two files, so a rename in the vocabulary would silently leave the demo with
// three unclassified positions, no asset-class drift and an empty decisions
// inbox: a broken feature that still records cleanly.
describe("the generated demo finance config", () => {
  const demoUp = readFileSync(
    fileURLToPath(new URL("./demo-up", import.meta.url)),
    "utf8",
  );

  test("names every canonical instrument the demo vocabulary declares", () => {
    for (const instrument of VOCABULARY.instruments) {
      expect(demoUp).toContain(`"instrument": "${instrument.canonical}"`);
    }
  });

  test("declares a target cohort that sums to ten thousand basis points", () => {
    // A cohort that does not add up emits a caveat and no drift proposal at all,
    // so this is what keeps the recorded demo's inbox non-empty.
    const targets = [...demoUp.matchAll(/"target_bp":\s*(\d+)/g)].map((match) =>
      Number(match[1]),
    );
    expect(targets.length).toBeGreaterThan(0);
    expect(targets.reduce((total, value) => total + value, 0)).toBe(10_000);
  });

  test("every declared asset class is one an instrument carries", () => {
    const classes = new Set(
      [...demoUp.matchAll(/"asset_class":\s*"([a-z_]+)"/g)].map((match) => match[1]),
    );
    const targeted = [...demoUp.matchAll(/"asset_class":\s*"([a-z_]+)",\n\s*"target_bp"/g)];
    for (const [, name] of targeted) {
      expect(classes.has(name)).toBe(true);
    }
  });
});

describe("activeOverlay", () => {
  test("reads a paths.sh whose directory name is a shell command", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-demo-seed-"));
    try {
      const marker = join(root, "INJECTED");
      // Every metacharacter that mattered, in one directory name: command substitution,
      // a quote to break out of the old `source "..."`, and a backtick.
      const hostile = join(root, `d"$(touch '${marker}')\`touch '${marker}'\`x`);
      mkdirSync(hostile, { recursive: true });
      const pathsSh = join(hostile, "paths.sh");
      // A stand-in for tools/lib/paths.sh: `source`d for one variable, which is all
      // activeOverlay reads out of it.
      writeFileSync(pathsSh, 'SJEL_OVERLAY_ROOT="/tmp/axon-demo-seed-overlay"\n');

      expect(activeOverlay(pathsSh)).toBe("/tmp/axon-demo-seed-overlay");
      expect(existsSync(marker)).toBe(false);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("fails loudly when paths.sh is not there, rather than reporting no overlay", () => {
    expect(() => activeOverlay(join(tmpdir(), "axon-demo-seed-absent", "paths.sh"))).toThrow();
  });
});
