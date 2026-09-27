import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { overlayRoot, resolveOverlayRoot } from "./overlay.ts";

let root = "";
let oldGeneric: string | undefined;
let oldLegacy: string | undefined;

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "axon-overlay-test-"));
  oldGeneric = process.env.SJEL_OVERLAY_ROOT;
  oldLegacy = process.env.SJEL_PERSONAL_ROOT;
  delete process.env.SJEL_OVERLAY_ROOT;
  delete process.env.SJEL_PERSONAL_ROOT;
});

afterEach(() => {
  rmSync(root, { recursive: true, force: true });
  if (oldGeneric === undefined) delete process.env.SJEL_OVERLAY_ROOT;
  else process.env.SJEL_OVERLAY_ROOT = oldGeneric;
  if (oldLegacy === undefined) delete process.env.SJEL_PERSONAL_ROOT;
  else process.env.SJEL_PERSONAL_ROOT = oldLegacy;
});

describe("overlay resolution", () => {
  test("prefers the canonical environment contract", () => {
    process.env.SJEL_OVERLAY_ROOT = "/generic";
    process.env.SJEL_PERSONAL_ROOT = "/legacy";
    expect(resolveOverlayRoot(root)).toEqual({ root: "/generic", source: "SJEL_OVERLAY_ROOT" });
  });
  test("keeps the historical environment alias compatible", () => {
    process.env.SJEL_PERSONAL_ROOT = "/legacy";
    expect(overlayRoot(root)).toBe("/legacy");
  });
  test("prefers the machine-local top-level key", () => {
    writeFileSync(join(root, "axon.local.toml"), 'overlay = "/local"\n');
    writeFileSync(join(root, "axon.toml"), '[platform]\noverlay = "/default"\n');
    expect(resolveOverlayRoot(root)).toEqual({ root: "/local", source: "axon.local.toml" });
  });
  test("reads the tracked platform fallback", () => {
    writeFileSync(join(root, "axon.toml"), '[platform]\noverlay = "/default"\n');
    expect(resolveOverlayRoot(root)).toEqual({ root: "/default", source: "axon.toml" });
  });
});
