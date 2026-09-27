import { existsSync, readFileSync } from "node:fs";
import { hostname } from "node:os";
import { dirname, resolve } from "node:path";

export type OverlaySource = "SJEL_OVERLAY_ROOT" | "SJEL_PERSONAL_ROOT" | "axon.local.toml" | "axon.toml";
export type OverlayResolution = { root: string; source: OverlaySource };

export function axonRoot(): string {
  return resolve(dirname(new URL(import.meta.url).pathname), "..", "..");
}

function expandHome(path: string): string {
  const home = process.env.HOME;
  if (!home) return path;
  if (path === "~") return home;
  return path.startsWith("~/") ? resolve(home, path.slice(2)) : path;
}

function readOverlayKey(file: string): string | null {
  let doc: { overlay?: unknown; platform?: { overlay?: unknown } };
  try {
    doc = Bun.TOML.parse(readFileSync(file, "utf8")) as typeof doc;
  } catch {
    return null;
  }
  for (const candidate of [doc.overlay, doc.platform?.overlay]) {
    if (typeof candidate === "string" && candidate.trim()) return candidate.trim();
  }
  return null;
}

export function resolveOverlayRoot(root: string = axonRoot()): OverlayResolution | null {
  for (const name of ["SJEL_OVERLAY_ROOT", "SJEL_PERSONAL_ROOT"] as const) {
    const value = process.env[name]?.trim();
    if (value) return { root: expandHome(value), source: name };
  }
  for (const name of ["axon.local.toml", "axon.toml"] as const) {
    const value = readOverlayKey(resolve(root, name));
    if (value) return { root: expandHome(value), source: name };
  }
  return null;
}

export function overlayRoot(root: string = axonRoot()): string | null {
  return resolveOverlayRoot(root)?.root ?? null;
}

export type MachineSource = "axon.local.toml" | "hostname" | "config/machine.toml";
export type MachineResolution = { path: string; source: MachineSource; name: string | null };

function readMachineKey(file: string): string | null {
  try {
    const doc = Bun.TOML.parse(readFileSync(file, "utf8")) as { machine?: unknown };
    return typeof doc.machine === "string" && doc.machine.trim() ? doc.machine.trim() : null;
  } catch {
    return null;
  }
}

/**
 * Which of an overlay's machines this host is.
 *
 * An overlay describes a deployment, and a deployment may own several machines — a
 * cheap always-on service node and a separate compute node, say. Reading the wrong
 * manifest means reading another machine's enabled set, container runtime and state
 * mounts, so the file is selected rather than assumed.
 *
 * This mirrors the resolution order in `tools/lib/paths.sh` exactly. Two
 * implementations exist because the shell tools cannot call this and forking a Bun
 * process per shell invocation is worse; both are covered by tests, and a change to
 * one that is not made to the other is a bug even when both still pass.
 *
 * Returns null when nothing resolves, which callers report rather than guessing at.
 */
export function resolveMachineToml(
  overlay: string,
  opts: { hostname?: string | null } = {},
): MachineResolution | null {
  const explicit = readMachineKey(resolve(axonRoot(), "axon.local.toml"));
  if (explicit) {
    // A named machine that does not exist is an error, never a fallback: falling back
    // would operate on a different machine while looking like it worked.
    return { path: resolve(overlay, "config", "machines", `${explicit}.toml`), source: "axon.local.toml", name: explicit };
  }
  // Short name either way. `hostname -s` is what paths.sh reads, and a caller passing
  // an FQDN should get the same answer as the host reporting one.
  const host = (opts.hostname ?? hostname()).split(".")[0];
  if (host) {
    const byHost = resolve(overlay, "config", "machines", `${host}.toml`);
    if (existsSync(byHost)) return { path: byHost, source: "hostname", name: host };
  }
  return { path: resolve(overlay, "config", "machine.toml"), source: "config/machine.toml", name: null };
}
