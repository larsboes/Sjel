// tools/updates.test.ts — planted-fixture regression test for tools/updates.ts.
//
// The three parsers are the part most likely to rot silently: the formats belong to cargo, npm,
// brew and rustup, not to us, and a shape change there turns a stale crate into a "current" row
// without anything failing. Each parser is driven against output captured from the real command
// on 2026-10-01, and the gatherers are driven through an injected runner so no test touches a
// registry or needs a network.
//
// The two claims worth guarding are about what this tool must NOT do: `apply` must never grow a
// second `brew upgrade` (one binary, one owner — tools/host-patch.sh's rule), and a pre-release
// that merely sorts higher must never be adopted. Both are asserted below rather than described.
//
// Run: bun test tools/updates.test.ts

import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  applyReceiptPath,
  buildReport,
  gatherCargo,
  gatherNpm,
  grouped,
  makeCtx,
  parseArgs,
  parseBrewFormulae,
  parseBrewOutdated,
  parseCargoInstallList,
  parseCargoSearch,
  parseCratesIoVersions,
  parseNpmGlobalTree,
  parseNpmDeprecated,
  parseNpmOutdated,
  parseReceipt,
  parseRustupCheck,
  planApply,
  readApplyReceipt,
  receiptNote,
  renderJson,
  renderTable,
  surface,
  SURFACES,
  versionNewer,
  writeApplyReceipt,
  type Ctx,
  type Runner,
} from "./updates.ts";

// Output captured from `cargo install --list` on this host, 2026-10-01. The indented lines are
// each crate's binaries; a parser that folded them in would report a crate named "btm".
const CARGO_LIST = `bottom v0.14.7:
    btm
macmon v0.7.0:
    macmon
tauri-cli v2.12.0:
    cargo-tauri
xberg-cli v1.0.14:
    xberg
`;

const NPM_OUTDATED = JSON.stringify({
  "@marckrenn/pi-sub-bar": { current: "1.4.0", wanted: "1.5.0", latest: "1.5.0" },
  pnpm: { current: "10.23.0", wanted: "12.8.1", latest: "12.8.1" },
});

const NPM_INSTALLED = JSON.stringify({
  dependencies: {
    "@marckrenn/pi-sub-bar": { version: "1.4.0" },
    "@earendil-works/pi-coding-agent": { version: "0.99.2" },
    uv: { version: "1.4.0" },
  },
});

/** The tree that made the tool recommend three upgrades it must not make. */
const NPM_TREE_CONSTRAINED = JSON.stringify({
  dependencies: {
    "claude-agent-sdk-pi": {
      version: "1.0.16",
      dependencies: {
        "@mariozechner/pi-coding-agent": {
          version: "0.52.12",
          dependencies: { "@mariozechner/pi-agent-core": { version: "0.52.12" } },
        },
      },
    },
    "@mariozechner/pi-agent-core": { version: "0.52.12" },
    defuddle: { version: "0.19.3" },
  },
});

const RUSTUP_CHECK = `stable-aarch64-apple-darwin - up to date: 1.99.0 (b940084d7 2026-09-28)
rustup - up to date : 1.29.1
`;

const RUSTUP_BEHIND = `stable-aarch64-apple-darwin - Update available : 1.98.0 -> 1.99.0
rustup - up to date : 1.29.1
`;

/** A runner that answers from a table and records what it was asked. */
function fakeRun(table: Record<string, string>, codes: Record<string, number> = {}) {
  const calls: string[][] = [];
  const run: Runner = (argv) => {
    calls.push(argv);
    const key = argv.join(" ");
    const hit = Object.keys(table).find((k) => key.startsWith(k));
    return { code: codes[key] ?? codes[argv[0]] ?? (hit ? 0 : 1), stdout: hit ? table[hit] : "", stderr: "" };
  };
  return { run, calls };
}

function ctxWith(table: Record<string, string>, opts: { offline?: boolean; codes?: Record<string, number> } = {}) {
  const { run, calls } = fakeRun(table, opts.codes);
  const ctx: Ctx = {
    run,
    // A binary is "installed" when the table has a command line that starts with it — the keys
    // are full argv strings, so `bin in table` would answer no for every one of them.
    have: (bin: string) =>
      Object.keys(table).some((k) => k === bin || k.startsWith(`${bin} `)) ? `/usr/bin/${bin}` : null,
    root: "/repo",
    overlay: "/overlay",
    offline: opts.offline ?? false,
  };
  return { ctx, calls };
}

describe("parseCargoInstallList", () => {
  test("reads one crate per stanza and ignores the binary lines", () => {
    expect(parseCargoInstallList(CARGO_LIST)).toEqual([
      { name: "bottom", version: "0.14.7" },
      { name: "macmon", version: "0.7.0" },
      { name: "tauri-cli", version: "2.12.0" },
      { name: "xberg-cli", version: "1.0.14" },
    ]);
  });

  test("tolerates a colon-less line older cargos printed", () => {
    expect(parseCargoInstallList("foo v1.2.3\n")).toEqual([{ name: "foo", version: "1.2.3" }]);
  });

  test("an empty install list is not a crate", () => {
    expect(parseCargoInstallList("")).toEqual([]);
  });
});

describe("parseCargoSearch", () => {
  test("reads the version for the crate asked about, not the first line", () => {
    const out = `other = "9.9.9"   # a different crate\nmacmon = "0.8.2"   # Apple Silicon monitor\n`;
    expect(parseCargoSearch("macmon", out)).toBe("0.8.2");
  });

  test("a crate name with regex characters does not break the lookup", () => {
    expect(parseCargoSearch("a.b+c", `a.b+c = "1.0.0"`)).toBe("1.0.0");
  });

  test("no line for the crate means null, not a wrong version", () => {
    expect(parseCargoSearch("macmon", `something-else = "1.0.0"`)).toBeNull();
  });
});

describe("npm parsers", () => {
  test("outdated reports current and latest per package", () => {
    expect(parseNpmOutdated(NPM_OUTDATED)).toEqual([
      { name: "@marckrenn/pi-sub-bar", current: "1.4.0", latest: "1.5.0" },
      { name: "pnpm", current: "10.23.0", latest: "12.8.1" },
    ]);
  });

  test("npm's non-JSON noise is swallowed rather than thrown", () => {
    // npm prints warnings on stderr and sometimes on stdout; a throw here would take the whole
    // report down over a package nothing is wrong with.
    expect(parseNpmOutdated("npm warn something\n")).toEqual([]);
  });

  test("installed reads the version map", () => {
    expect(parseNpmGlobalTree(NPM_INSTALLED).installed.map((p) => p.name)).toEqual([
      "@marckrenn/pi-sub-bar",
      "@earendil-works/pi-coding-agent",
      "uv",
    ]);
  });

  test("a flat tree constrains nothing", () => {
    expect([...parseNpmGlobalTree(NPM_INSTALLED).requiredBy]).toEqual([]);
  });

  // The distinction the first version of this missed, measured on this host 2026-10-01:
  // `@marckrenn/pi-sub-bar@1.5.0` bundles its OWN `@mariozechner/pi-coding-agent@0.73.1`, so the
  // top-level 0.52.12 satisfies nobody. Matching by name alone called that "pinned"; the version
  // is what says it is a leftover.
  test("a parent that resolved a different version did not pin the top-level copy", () => {
    const tree = JSON.stringify({
      dependencies: {
        "@marckrenn/pi-sub-bar": {
          version: "1.5.0",
          dependencies: { "@mariozechner/pi-coding-agent": { version: "0.73.1", dependencies: { "@mariozechner/pi-agent-core": { version: "0.73.1" } } } },
        },
        "@mariozechner/pi-agent-core": { version: "0.52.12" },
      },
    });
    expect(parseNpmGlobalTree(tree).requiredBy.get("@mariozechner/pi-agent-core")).toEqual([
      { parent: "@marckrenn/pi-sub-bar", version: "0.73.1" },
    ]);
  });

  // The defect this exists to stop, captured from the real tree on 2026-10-01:
  // `@marionzechner/pi-agent-core` and `@sinclair/typebox` are top-level only because two other
  // globals hoisted them, and `npm outdated -g` cheerfully offered a version neither range admits.
  test("a package another global requires is reported as pinned, with its parents", () => {
    const tree = JSON.stringify({
      dependencies: {
        "claude-agent-sdk-pi": {
          version: "1.0.16",
          dependencies: { "@mariozechner/pi-coding-agent": { version: "0.52.12", dependencies: { "@mariozechner/pi-agent-core": { version: "0.52.12" } } } },
        },
        "@marckrenn/pi-sub-bar": {
          version: "1.4.0",
          dependencies: { "@sinclair/typebox": { version: "0.34.48" } },
        },
        "@mariozechner/pi-agent-core": { version: "0.52.12" },
        "@sinclair/typebox": { version: "0.34.48" },
        "defuddle": { version: "0.19.3" },
      },
    });
    const { requiredBy } = parseNpmGlobalTree(tree);
    expect(requiredBy.get("@mariozechner/pi-agent-core")).toEqual([
      { parent: "claude-agent-sdk-pi", version: "0.52.12" },
    ]);
    expect(requiredBy.get("@sinclair/typebox")).toEqual([{ parent: "@marckrenn/pi-sub-bar", version: "0.34.48" }]);
    // A package nothing else requires stays independently upgradable.
    expect(requiredBy.has("defuddle")).toBe(false);
    // A parent is not "required by" the child it requires.
    expect(requiredBy.has("claude-agent-sdk-pi")).toBe(false);
  });

  test("a broken tree still yields what it could read", () => {
    expect(parseNpmGlobalTree("npm error unmet").installed).toEqual([]);
  });
});

describe("parseNpmDeprecated", () => {
  // Captured 2026-10-01: the whole @mariozechner scope was renamed, and npm says so.
  test("reads the registry's own deprecation message", () => {
    expect(parseNpmDeprecated("please use @earendil-works/pi-agent-core instead going forward")).toBe(
      "please use @earendil-works/pi-agent-core instead going forward",
    );
  });

  test("a live package says nothing, and npm's own errors are not a deprecation", () => {
    expect(parseNpmDeprecated("")).toBeNull();
    expect(parseNpmDeprecated("npm error code E404")).toBeNull();
    expect(parseNpmDeprecated("npm warn something")).toBeNull();
  });
});

describe("parseBrewOutdated", () => {
  test("empty streams are the normal 'nothing outdated' answer", () => {
    expect(parseBrewOutdated(`{"formulae":[],"casks":[]}`)).toEqual([]);
  });

  test("formulae and casks are both read, and a cask says so", () => {
    const json = JSON.stringify({
      formulae: [{ name: "nettle", installed_versions: ["3.10"], current_version: "3.10.1" }],
      casks: [{ name: "tuist", installed_versions: ["1.0"], current_version: "1.1" }],
    });
    expect(parseBrewOutdated(json)).toEqual([
      { name: "nettle", installed: "3.10", latest: "3.10.1" },
      { name: "tuist (cask)", installed: "1.0", latest: "1.1" },
    ]);
  });
});

describe("parseRustupCheck", () => {
  test("up to date lines carry a version and no latest", () => {
    expect(parseRustupCheck(RUSTUP_CHECK)).toEqual([
      { component: "stable-aarch64-apple-darwin", installed: "1.99.0" },
      { component: "rustup", installed: "1.29.1" },
    ]);
  });

  test("an available update is read off the arrow", () => {
    const [first] = parseRustupCheck(RUSTUP_BEHIND);
    expect(first).toEqual({ component: "stable-aarch64-apple-darwin", installed: "1.98.0", latest: "1.99.0" });
  });
});

describe("versionNewer", () => {
  test("compares numerically, not as strings", () => {
    // The bug this exists to stop: "0.14.9" < "0.14.7" as a string.
    expect(versionNewer("0.14.9", "0.14.7")).toBe(true);
    expect(versionNewer("1.3.0", "1.0.14")).toBe(true);
    expect(versionNewer("0.14.7", "0.14.9")).toBe(false);
  });

  test("a release beats its own pre-release", () => {
    expect(versionNewer("2.13.0", "2.13.0-beta.1")).toBe(true);
    expect(versionNewer("2.13.0-beta.1", "2.13.0")).toBe(false);
  });

  test("a leading v is not a difference", () => {
    expect(versionNewer("v1.2.3", "1.2.3")).toBe(false);
  });
});

describe("receipts", () => {
  test("an unreadable receipt is null rather than a silently fresh one", () => {
    expect(parseReceipt("not json")).toBeNull();
  });

  test("the note carries the job's age and the audit's verdict, which are different facts", () => {
    const note = receiptNote({ ageH: 3, audit: "finding", failed: "", ran: "brew update" });
    expect(note).toContain("job last ran 3h ago");
    expect(note).toContain("audit finding — run tools/audit");
  });
});

describe("parseCratesIoVersions", () => {
  const versions = (list: { num: string; yanked?: boolean }[]) => JSON.stringify({ versions: list, meta: {} });

  // The defect this exists to fix: `cargo search` reports only the maximum, so tauri-cli's
  // released patch 2.12.1 was invisible behind 3.0.0-alpha.4.
  test("finds the newest stable even when a pre-release sorts higher", () => {
    const json = versions([
      { num: "3.0.0-alpha.4" },
      { num: "2.12.1" },
      { num: "2.12.0" },
      { num: "2.11.5" },
    ]);
    expect(parseCratesIoVersions(json)).toEqual({ stable: "2.12.1", newest: "3.0.0-alpha.4" });
  });

  test("compares numerically, not lexically", () => {
    const json = versions([{ num: "2.9.0" }, { num: "2.10.0" }, { num: "2.12.1" }]);
    expect(parseCratesIoVersions(json).stable).toBe("2.12.1");
  });

  test("a yanked release is not a release to move to", () => {
    const json = versions([{ num: "2.12.1", yanked: true }, { num: "2.12.0" }]);
    expect(parseCratesIoVersions(json).stable).toBe("2.12.0");
  });

  test("pre-releases only means no stable, and newest still names one", () => {
    expect(parseCratesIoVersions(versions([{ num: "1.0.0-beta.1" }, { num: "0.9.0-rc.2" }]))).toEqual({
      stable: null,
      newest: "1.0.0-beta.1",
    });
  });

  test("an error page or empty list is no answer, not a wrong one", () => {
    expect(parseCratesIoVersions("<html>403</html>")).toEqual({ stable: null, newest: null });
    expect(parseCratesIoVersions(versions([]))).toEqual({ stable: null, newest: null });
  });
});

describe("gatherCargo", () => {
  test("a stale crate names the exact command that moves it", () => {
    const { ctx } = ctxWith({
      "cargo install --list": CARGO_LIST,
      "curl -sS": JSON.stringify({ versions: [{ num: "0.8.2" }] }),
    });
    const rows = gatherCargo(ctx);
    // Every crate gets the same API answer here, so macmon is the one to read.
    const macmon = rows.find((r) => r.name === "macmon")!;
    expect(macmon.status).toBe("stale");
    expect(macmon.installed).toBe("0.7.0");
    expect(macmon.latest).toBe("0.8.2");
    expect(macmon.action).toBe("cargo install macmon --locked --force");
  });

  // Measured on this host: `cargo search tauri-cli` answers 3.0.0-alpha.4 while 2.12.1 — a
  // released patch above the installed 2.12.0 — is what should move. The alpha is named, the
  // stable patch is the action, and both facts appear on one row.
  test("a stable patch is adopted while the alpha above it is only named", () => {
    const { ctx } = ctxWith({
      "cargo install --list": `tauri-cli v2.12.0:\n    cargo-tauri\n`,
      "curl -sS": JSON.stringify({ versions: [{ num: "3.0.0-alpha.4" }, { num: "2.12.1" }, { num: "2.12.0" }] }),
    });
    const [row] = gatherCargo(ctx);
    expect(row.status).toBe("stale");
    expect(row.latest).toBe("2.12.1");
    expect(row.action).toBe("cargo install tauri-cli --locked --force");
    expect(row.note).toContain("3.0.0-alpha.4");
    expect(row.note).toContain("not adopted");
  });

  test("when only pre-releases exist, nothing is adopted", () => {
    const { ctx } = ctxWith({
      "cargo install --list": `tauri-cli v2.12.0:\n    cargo-tauri\n`,
      "curl -sS": JSON.stringify({ versions: [{ num: "3.0.0-alpha.4" }] }),
    });
    const [row] = gatherCargo(ctx);
    expect(row.status).toBe("current");
    expect(row.action).toBeUndefined();
    expect(row.note).toContain("pre-release");
  });

  test("an unreachable crates.io falls back to cargo search", () => {
    const { ctx } = ctxWith({
      "cargo install --list": `macmon v0.7.0:\n    macmon\n`,
      "cargo search macmon": `macmon = "0.8.2"`,
    });
    const [row] = gatherCargo(ctx);
    expect(row.status).toBe("stale");
    expect(row.latest).toBe("0.8.2");
  });

  test("a registry that does not answer is unknown, never current", () => {
    const { ctx } = ctxWith({ "cargo install --list": CARGO_LIST }, { codes: { "cargo search macmon": 1 } });
    const macmon = gatherCargo(ctx).find((r) => r.name === "macmon")!;
    expect(macmon.status).toBe("unknown");
  });

  test("--offline reports the installed version and claims nothing about newer ones", () => {
    const { ctx, calls } = ctxWith({ "cargo install --list": CARGO_LIST }, { offline: true });
    const rows = gatherCargo(ctx);
    expect(rows.every((r) => r.status === "unknown")).toBe(true);
    expect(calls.some((c) => c[0] === "cargo" && c[1] === "search")).toBe(false);
  });
});

describe("gatherNpm", () => {
  test("a package on latest still appears, so the list is an inventory and not a diff", () => {
    const { ctx } = ctxWith({ "npm ls -g": NPM_INSTALLED, "npm outdated -g": NPM_OUTDATED });
    const rows = gatherNpm(ctx, new Set());
    const pi = rows.find((r) => r.name === "@earendil-works/pi-coding-agent")!;
    expect(pi.status).toBe("current");
    expect(pi.installed).toBe("0.99.2");
  });

  test("a name that is also a brew formula is flagged to check PATH", () => {
    const { ctx } = ctxWith({ "npm ls -g": NPM_INSTALLED, "npm outdated -g": "{}" });
    const rows = gatherNpm(ctx, parseBrewFormulae("uv\njq\nnettle\n"));
    expect(rows.find((r) => r.name === "uv")!.note).toContain("brew formula");
    expect(rows.find((r) => r.name === "@marckrenn/pi-sub-bar")!.note).toBeUndefined();
  });

  // The regression that matters: a stale row with no action, naming the parent to upgrade
  // instead. Without this the tool told the reader to install a version its parent's range
  // does not admit.
  test("a package another global pins is stale, names its parent, and carries no action", () => {
    const { ctx } = ctxWith({
      "npm ls -g": NPM_TREE_CONSTRAINED,
      "npm outdated -g": JSON.stringify({
        "@mariozechner/pi-agent-core": { current: "0.52.12", latest: "0.73.1" },
        defuddle: { current: "0.19.3", latest: "0.19.4" },
      }),
    });
    const rows = gatherNpm(ctx, new Set());
    const pinned = rows.find((r) => r.name === "@mariozechner/pi-agent-core")!;
    expect(pinned.status).toBe("stale");
    expect(pinned.action).toBeUndefined();
    expect(pinned.note).toContain("pinned by claude-agent-sdk-pi");
    expect(pinned.note).toContain("upgrade those instead");

    // The unconstrained one beside it is still actionable.
    const free = rows.find((r) => r.name === "defuddle")!;
    expect(free.status).toBe("stale");
    expect(free.action).toBe("npm install -g defuddle@latest");
  });

  // The leftover case: nothing requires this copy at this version. Upgrading would install a
  // third copy, so the row says so and offers nothing — removal is the reader's call.
  test("a duplicate nothing requires is named as unused and carries no action", () => {
    const { ctx } = ctxWith({
      "npm ls -g": JSON.stringify({
        dependencies: {
          "@marckrenn/pi-sub-bar": {
            version: "1.5.0",
            dependencies: { "@mariozechner/pi-coding-agent": { version: "0.73.1", dependencies: { "@mariozechner/pi-agent-core": { version: "0.73.1" } } } },
          },
          "@mariozechner/pi-agent-core": { version: "0.52.12" },
        },
      }),
      "npm outdated -g": JSON.stringify({ "@mariozechner/pi-agent-core": { current: "0.52.12", latest: "0.73.1" } }),
    });
    const row = gatherNpm(ctx, new Set()).find((r) => r.name === "@mariozechner/pi-agent-core")!;
    expect(row.status).toBe("stale");
    expect(row.action).toBeUndefined();
    expect(row.note).toContain("unused duplicate");
    expect(row.note).toContain("bundles its own 0.73.1");
  });

  // The second half of the same lesson: npm's newest version is not always the right version.
  // The @mariozechner scope was renamed to @earendil-works, so its 0.73.1 is deprecated rather
  // than an upgrade, and the tool must not offer to install it.
  test("a deprecated package is named as deprecated and carries no action", () => {
    const { ctx } = ctxWith({
      "npm ls -g": NPM_INSTALLED,
      "npm outdated -g": JSON.stringify({ uv: { current: "1.4.0", latest: "1.5.0" } }),
      "npm view uv deprecated": "please use @earendil-works/uv instead going forward",
    });
    const uv = gatherNpm(ctx, new Set()).find((r) => r.name === "uv")!;
    expect(uv.status).toBe("stale");
    expect(uv.action).toBeUndefined();
    expect(uv.note).toContain("deprecated");
    expect(uv.note).toContain("@earendil-works/uv");
  });

  test("a live package is still offered, and is asked about deprecation only once", () => {
    const { ctx, calls } = ctxWith({
      "npm ls -g": NPM_INSTALLED,
      "npm outdated -g": JSON.stringify({ uv: { current: "1.4.0", latest: "1.5.0" } }),
    });
    const uv = gatherNpm(ctx, new Set()).find((r) => r.name === "uv")!;
    expect(uv.action).toBe("npm install -g uv@latest");
    expect(calls.filter((c) => c[0] === "npm" && c[1] === "view").length).toBe(1);
  });

  test("a pinned package is never planned as an apply step", () => {
    const { ctx } = ctxWith({
      "npm ls -g": NPM_TREE_CONSTRAINED,
      "npm outdated -g": JSON.stringify({ "@mariozechner/pi-agent-core": { current: "0.52.12", latest: "0.73.1" } }),
    });
    const steps = planApply(gatherNpm(ctx, new Set()), ["npm"], ctx);
    expect(steps).toEqual([]);
  });
});

describe("planApply — the one-owner rule", () => {
  const stale = [
    { surface: "brew", name: "nettle", owner: "scheduled" as const, ownerDetail: "", status: "stale" as const },
    { surface: "cargo", name: "macmon", owner: "unowned" as const, ownerDetail: "", status: "stale" as const, action: "cargo install macmon --locked --force" },
    { surface: "npm", name: "pnpm", owner: "unowned" as const, ownerDetail: "", status: "stale" as const, action: "npm install -g pnpm@latest" },
    { surface: "graphify", name: "harness integration", owner: "manual" as const, ownerDetail: "", status: "stale" as const, action: "tools/agent-integrations.sh update graphify" },
  ];
  const ctx = { run: () => ({ code: 0, stdout: "", stderr: "" }), have: () => null, root: "/repo", overlay: "/overlay", offline: false };

  test("brew is moved by its owner, never by a command this tool invents", () => {
    const steps = planApply(stale, [], ctx);
    const brewStep = steps.find((s) => s.surfaceId === "hostpatch")!;
    expect(brewStep.argv).toEqual(["/repo/tools/host-patch.sh"]);
    // The regression this guards: a future edit adding `brew upgrade` here directly.
    const allArgv = steps.flatMap((s) => s.argv).join(" ");
    expect(allArgv).not.toContain("brew upgrade");
    expect(allArgv).not.toContain("uv tool");
    expect(allArgv).not.toContain("rustup update");
  });

  test("the unowned classes are moved directly, because nothing else will", () => {
    const steps = planApply(stale, [], ctx);
    expect(steps.find((s) => s.surfaceId === "cargo")!.argv).toEqual(["cargo", "install", "macmon", "--locked", "--force"]);
    expect(steps.find((s) => s.surfaceId === "npm")!.argv).toEqual(["npm", "install", "-g", "pnpm@latest"]);
  });

  test("the integration is delegated to the verb that owns it", () => {
    const steps = planApply(stale, [], ctx);
    expect(steps.find((s) => s.surfaceId === "graphify")!.argv).toEqual([
      "/repo/tools/agent-integrations.sh",
      "update",
      "graphify",
    ]);
  });

  test("--only scopes the plan and leaves the rest alone", () => {
    const steps = planApply(stale, ["cargo"], ctx);
    expect(steps.map((s) => s.surfaceId)).toEqual(["cargo"]);
  });

  test("nothing stale plans nothing, so apply cannot run a job for no reason", () => {
    expect(planApply([], [], ctx)).toEqual([]);
  });
});

describe("parseArgs", () => {
  test("a leading flag is the default verb, not a verb named --offline", () => {
    expect(parseArgs(["--offline"]).verb).toBe("report");
    expect(parseArgs(["--offline"]).offline).toBe(true);
    expect(parseArgs([]).verb).toBe("report");
  });

  test("apply with flags after it parses", () => {
    const o = parseArgs(["apply", "--only", "cargo,npm", "--yes"]);
    expect(o.verb).toBe("apply");
    expect(o.only).toEqual(["cargo", "npm"]);
    expect(o.yes).toBe(true);
  });

  test("--only consumes exactly one argument", () => {
    expect(parseArgs(["apply", "--only", "cargo"]).only).toEqual(["cargo"]);
    expect(() => parseArgs(["apply", "--only"])).toThrow();
  });

  test("an unknown flag is an error rather than a silent no-op", () => {
    expect(() => parseArgs(["report", "--jsoon"])).toThrow();
  });
});

describe("makeCtx", () => {
  test("refuses to guess a root, because the overlay decides where the receipt is", () => {
    expect(() => makeCtx({}, () => ({ code: 0, stdout: "", stderr: "" }), () => null)).toThrow();
    const ctx = makeCtx({ SJEL_ROOT: "/r", SJEL_OVERLAY_ROOT: "/o" }, () => ({ code: 0, stdout: "", stderr: "" }), () => null);
    expect(ctx.root).toBe("/r");
    expect(ctx.overlay).toBe("/o");
  });
});

describe("the apply receipt", () => {
  // The dashboard's apply button cannot hold a request open while cargo compiles for minutes, so
  // the receipt is how a caller that did not wait learns the outcome. A round trip and a missing
  // file are the two states the panel has to survive.
  test("round-trips, and a missing receipt is null rather than a fabricated one", () => {
    const overlay = mkdtempSync(join(tmpdir(), "updates-receipt-"));
    try {
      expect(readApplyReceipt(overlay)).toBeNull();
      writeApplyReceipt(overlay, { at: "2026-10-01T00:00:00Z", class: "cargo", steps: 3, state: "running" });
      expect(readApplyReceipt(overlay)).toEqual({
        at: "2026-10-01T00:00:00Z",
        class: "cargo",
        steps: 3,
        state: "running",
      });
      expect(applyReceiptPath(overlay)).toContain("data/updates/last-apply.json");
    } finally {
      rmSync(overlay, { recursive: true, force: true });
    }
  });

  test("a corrupt receipt reads as absent, because the report must still render", () => {
    const overlay = mkdtempSync(join(tmpdir(), "updates-receipt-"));
    try {
      writeApplyReceipt(overlay, { at: "x", class: "cargo", state: "running" });
      writeFileSync(applyReceiptPath(overlay), "{ not json");
      expect(readApplyReceipt(overlay)).toBeNull();
    } finally {
      rmSync(overlay, { recursive: true, force: true });
    }
  });

  test("the json payload carries lastApply so a panel never needs a second endpoint", () => {
    const { ctx } = ctxWith({ "git -C /repo rev-list": "0\n", "git -C /repo status": "" }, { offline: true });
    const { rows, generatedAt } = buildReport(ctx);
    const payload = JSON.parse(renderJson(rows, generatedAt, true, { at: "2026-10-01T00:00:00Z", class: "npm", state: "done", steps: 11 }));
    expect(payload.lastApply.class).toBe("npm");
    expect(payload.lastApply.steps).toBe(11);
  });

  test("a running apply is visible in the table, and a finished one says how many steps", () => {
    const { ctx } = ctxWith({ "git -C /repo rev-list": "0\n", "git -C /repo status": "" }, { offline: true });
    const { rows } = buildReport(ctx);
    expect(renderTable(rows, false, { at: new Date().toISOString(), class: "cargo", state: "running" })).toContain(
      "an apply is running: cargo",
    );
    expect(renderTable(rows, false, { at: "2026-10-01T00:00:00Z", class: "npm", state: "done", steps: 11, failed: 1 })).toContain(
      "last apply: npm done · 11 step(s), 1 failed",
    );
  });
});

describe("the report itself", () => {
  test("every surface has an owner and a reason, and the two unowned ones are actionable", () => {
    for (const s of SURFACES) {
      expect(s.why.length).toBeGreaterThan(10);
      expect(s.ownerDetail.length).toBeGreaterThan(3);
    }
    // The classes nothing moves are exactly the ones this tool must move.
    expect(SURFACES.filter((s) => s.owner === "unowned").map((s) => s.id)).toEqual(["cargo", "npm"]);
    for (const s of SURFACES.filter((x) => x.owner === "unowned")) expect(s.actionable).toBe(true);
    // A vendor or self-managed class must never be actionable: that is the two-owners failure.
    for (const s of SURFACES.filter((x) => x.owner === "self")) expect(s.actionable).toBe(false);
  });

  test("an unknown class is refused by name", () => {
    expect(() => surface("apt")).toThrow(/unknown class/);
  });

  test("a report over planted input groups every row under a known surface", () => {
    const { ctx } = ctxWith(
      {
        "cargo install --list": CARGO_LIST,
        "npm ls -g": NPM_INSTALLED,
        "npm outdated -g": NPM_OUTDATED,
        "git -C /repo rev-list": "0\n",
        "git -C /repo status": "",
      },
      { offline: true },
    );
    const { rows } = buildReport(ctx);
    const ids = new Set(SURFACES.map((s) => s.id));
    for (const r of rows) expect(ids.has(r.surface)).toBe(true);
    // The grouping renderer must not silently drop a row it cannot place.
    const groupedCount = grouped(rows).reduce((n, g) => n + g.rows.length, 0);
    expect(groupedCount).toBe(rows.length);
  });

  test("the table names the owner's command for every stale row", () => {
    const { ctx } = ctxWith({
      "cargo install --list": `macmon v0.7.0:\n    macmon\n`,
      "cargo search macmon": `macmon = "0.8.2"`,
      "npm ls -g": NPM_INSTALLED,
      "npm outdated -g": "{}",
      "git -C /repo rev-list": "0\n",
      "git -C /repo status": "",
    });
    const { rows, generatedAt } = buildReport(ctx);
    const table = renderTable(rows, false);
    const stale = rows.filter((r) => r.status === "stale");
    expect(stale.length).toBeGreaterThan(0);
    // A stale row either names the command that moves it, or names the parent that pins it and
    // therefore why there is no command. Anything else is a dead end for the reader.
    for (const r of stale) {
      const explained =
        Boolean(r.action) ||
        /pinned by .+ — upgrade those instead/.test(r.note ?? "") ||
        /unused duplicate — /.test(r.note ?? "") ||
        /^deprecated — /.test(r.note ?? "");
      expect(explained, `${r.surface} ${r.name} is stale with neither an action nor a reason`).toBe(true);
    }
    expect(table).toContain("Unowned — nothing moves these");

    const json = JSON.parse(renderJson(rows, generatedAt, false));
    expect(json.rows.length).toBe(rows.length);
    expect(json.surfaces.length).toBe(SURFACES.length);
    expect(json.generatedAt).toBe(generatedAt);
  });
});
