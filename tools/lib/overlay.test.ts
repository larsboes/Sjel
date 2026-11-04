import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { overlayRoot, resolveMachineToml, resolveOverlayRoot } from "./overlay.ts";

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

function scratchOverlay(): string {
  const overlay = mkdtempSync(join(tmpdir(), "overlay-machine-"));
  mkdirSync(join(overlay, "config", "machines"), { recursive: true });
  return overlay;
}

function writeManifest(path: string): void {
  writeFileSync(path, 'os = "linux"\ncontainer_runtime = "docker"\n');
}

describe("resolveMachineToml", () => {
  test("falls back to the single-file layout when nothing else matches", () => {
    const overlay = scratchOverlay();
    writeManifest(join(overlay, "config", "machine.toml"));

    const resolved = resolveMachineToml(overlay, { hostname: "no-such-host" });

    expect(resolved?.source).toBe("config/machine.toml");
    expect(resolved?.path).toBe(join(overlay, "config", "machine.toml"));
    expect(resolved?.name).toBeNull();
  });

  test("prefers a manifest named after the host", () => {
    const overlay = scratchOverlay();
    writeManifest(join(overlay, "config", "machine.toml"));
    writeManifest(join(overlay, "config", "machines", "service-node.toml"));

    const resolved = resolveMachineToml(overlay, { hostname: "service-node" });

    expect(resolved?.source).toBe("hostname");
    expect(resolved?.name).toBe("service-node");
    expect(resolved?.path).toBe(join(overlay, "config", "machines", "service-node.toml"));
  });

  test("ignores a hostname that has no manifest rather than inventing a path", () => {
    const overlay = scratchOverlay();
    writeManifest(join(overlay, "config", "machine.toml"));
    writeManifest(join(overlay, "config", "machines", "compute-node.toml"));

    const resolved = resolveMachineToml(overlay, { hostname: "service-node" });

    expect(resolved?.source).toBe("config/machine.toml");
  });

  test("strips the domain from a fully qualified hostname", () => {
    const overlay = scratchOverlay();
    writeManifest(join(overlay, "config", "machines", "service-node.toml"));

    const resolved = resolveMachineToml(overlay, { hostname: "service-node.fritz.box" });

    expect(resolved?.source).toBe("hostname");
    expect(resolved?.name).toBe("service-node");
  });

  test("two machines in one overlay resolve independently", () => {
    const overlay = scratchOverlay();
    writeManifest(join(overlay, "config", "machines", "service-node.toml"));
    writeManifest(join(overlay, "config", "machines", "compute-node.toml"));

    const service = resolveMachineToml(overlay, { hostname: "service-node" });
    const compute = resolveMachineToml(overlay, { hostname: "compute-node" });

    expect(service?.path).not.toBe(compute?.path);
    expect(service?.name).toBe("service-node");
    expect(compute?.name).toBe("compute-node");
  });
});
