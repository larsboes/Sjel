import { expect, test } from "bun:test";
import { rustImpact, type Package } from "./ci-impact.ts";

const packages: Package[] = [
  { name: "base", dir: "libs/base", dependencies: [] },
  { name: "app", dir: "apps/app", dependencies: ["base"] },
  { name: "other", dir: "libs/other", dependencies: [] },
  { name: "top", dir: "apps/top", dependencies: ["app"] },
];

test("includes transitive reverse dependencies", () => {
  expect(rustImpact(["libs/base/src/lib.rs"], packages).packages).toEqual(["app", "base", "top"]);
});

test("widens unknown and build inputs", () => {
  for (const file of ["Cargo.lock", "libs/base/build.rs", "libs/base/fixture.json"]) {
    expect(rustImpact([file], packages).mode).toBe("all");
  }
});

test("does not confuse sibling path prefixes", () => {
  expect(rustImpact(["libs/base-extra/src/lib.rs"], packages).mode).toBe("all");
});

test("reports unaffected packages explicitly", () => {
  expect(rustImpact(["libs/base/src/lib.rs"], packages).skipped).toEqual(["other"]);
});
