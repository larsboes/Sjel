// tools/ci-changes.test.ts — the GPUI input set derived from this repository, and the match.
// Run: bun test tools/ci-changes.test.ts

import { describe, expect, test } from "bun:test";
import { firstHit, gpuiInputs, upwardLiterals } from "./ci-changes.ts";

describe("upwardLiterals", () => {
  test("finds include paths and build-script joins, not ordinary strings", () => {
    const source = `include_str!("../../self.json"); dir.join("../../docs"); concat!(env!("X"), "/../y"); let s = "a/b";`;
    expect(upwardLiterals(source)).toEqual(["../../self.json", "../../docs", "../y"]);
  });
});

describe("this repository's GPUI inputs", () => {
  const inputs = gpuiInputs(`${import.meta.dir}/..`);
  const runs = (file: string) => firstHit([file], inputs) !== null;

  test("a path dependency added through Cargo.toml is an input", () => {
    expect(runs("libs/clip-socket/src/lib.rs")).toBe(true);
    expect(runs("libs/sjel-config/src/lib.rs")).toBe(true);
  });

  test("files compiled in from outside the crates are inputs", () => {
    expect(runs("self.json")).toBe(true);
    expect(runs("docs/plans/gpui.md")).toBe(true);
    expect(runs("dashboard/src/lib/nav.ts")).toBe(true);
    expect(runs("Cargo.toml")).toBe(true);
  });

  test("a capability, a pack or the plan is not", () => {
    expect(runs("capabilities/comms/src/main.rs")).toBe(false);
    expect(runs("Packs/knowledge/README.md")).toBe(false);
    expect(runs("PLAN.md")).toBe(false);
    expect(runs("apps/gpuix/file")).toBe(false);
  });
});
