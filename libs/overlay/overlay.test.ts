import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { resolveMachineToml } from "./overlay.ts";

// The TypeScript half of machine resolution. tools/lib/paths.sh implements the same
// rule for the shell tools, and tools/machine-resolution.test.sh covers that one; the
// two must agree, so these cases deliberately mirror it. A change made to one
// implementation and not the other is a bug even while both test files still pass.
//
// The explicit-name case reads the real repository's sjel.local.toml and so cannot be
// forced from here — it is covered on the shell side, where the file's location is a
// test input rather than a fixed path.

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
