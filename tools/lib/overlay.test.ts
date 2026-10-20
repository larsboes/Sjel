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
    writeFileSync(join(root, "sjel.local.toml"), 'overlay = "/local"\n');
    writeFileSync(join(root, "sjel.toml"), '[platform]\noverlay = "/default"\n');
    expect(resolveOverlayRoot(root)).toEqual({ root: "/local", source: "sjel.local.toml" });
  });
  test("prefers sjel.local.toml over the pre-rename axon.local.toml", () => {
    writeFileSync(join(root, "sjel.local.toml"), 'overlay = "/sjel-local"\n');
    writeFileSync(join(root, "axon.local.toml"), 'overlay = "/axon-local"\n');
    expect(resolveOverlayRoot(root)).toEqual({ root: "/sjel-local", source: "sjel.local.toml" });
  });
  test("still reads axon.local.toml alone, the pre-rename name", () => {
    writeFileSync(join(root, "axon.local.toml"), 'overlay = "/axon-local"\n');
    expect(resolveOverlayRoot(root)).toEqual({ root: "/axon-local", source: "axon.local.toml" });
  });
  test("prefers sjel.toml over the pre-rename axon.toml", () => {
    writeFileSync(join(root, "sjel.toml"), '[platform]\noverlay = "/sjel-default"\n');
    writeFileSync(join(root, "axon.toml"), '[platform]\noverlay = "/old-default"\n');
    expect(resolveOverlayRoot(root)).toEqual({ root: "/sjel-default", source: "sjel.toml" });
  });
  test("reads the tracked platform fallback", () => {
    writeFileSync(join(root, "sjel.toml"), '[platform]\noverlay = "/default"\n');
    expect(resolveOverlayRoot(root)).toEqual({ root: "/default", source: "sjel.toml" });
  });
  test("reads a checkout that only carries axon.toml", () => {
    writeFileSync(join(root, "axon.toml"), '[platform]\noverlay = "/old-default"\n');
    expect(resolveOverlayRoot(root)).toEqual({ root: "/old-default", source: "axon.toml" });
  });
});
