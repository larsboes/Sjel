// tools/doctor.test.ts — planted-fixture regression test for the two pure
// checks doctor.ts's "Systems (systems.toml)" section and "Undeclared
// connections" sweep are built on. Guards against the silent-green failure
// mode a grep-pattern sweep is otherwise prone to (pattern typo, path
// convention change) — see CONTRIBUTING.md#documentation-stays-owned-and-current.
// Run: bun test tools/doctor.test.ts

import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  backupAgeState,
  checkStateMountCoverage,
  classifyScheduledProducer,
  formatAge,
  parseLaunchdJobs,
  parseLaunchdSchedule,
  classifyArchiveAtTarget,
  attemptFinding,
  parseReceiptTimestamp,
  classifyProbeOutcome,
  resolveProbeTargets,
  PROBE_TIMEOUT_MS,
  collectWhyBlocks,
  findDanglingDecisionRefs,
  extractSiblingRepoRefs,
  findDecisionPathRot,
  findPlaintextSecretsInEnvTemplate,
  parseEnvTemplateLines,
  formatFetchAge,
  formatVersion,
  findProductionListenerConstructs,
  findRustSources,
  isSweepExempt,
  stripRustCfgTestItems,
  whyBlockBases,
} from "./doctor.ts";

describe("production Rust server policy", () => {
  test("nested binary roots remain inside the bind-policy scan", () => {
    const root = mkdtempSync(join(tmpdir(), "axon-doctor-rust-"));
    try {
      mkdirSync(join(root, "server"));
      writeFileSync(join(root, "lib.rs"), "pub fn library() {}\n");
      writeFileSync(join(root, "server", "main.rs"), "fn main() {}\n");
      writeFileSync(join(root, "server", "notes.txt"), "not Rust\n");

      expect(findRustSources(root).map((path) => path.slice(root.length + 1))).toEqual([
        "lib.rs",
        "server/main.rs",
      ]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test("test-only listener constructs are excluded", () => {
    const source = `
fn build_router() -> Router { Router::new() }
fn main() { sjel_server::serve_local("fixture", 1234, build_router()); }

#[cfg(test)]
mod tests {
  async fn serve() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    axum::serve(listener, build_router()).await.unwrap();
  }
}`;

    expect(findProductionListenerConstructs(source)).toEqual([]);
    expect(stripRustCfgTestItems(source)).toContain("sjel_server::serve_local");
  });

  test("production listener constructs remain findings", () => {
    const source = `
fn build_router() -> Router { Router::new() }
async fn main() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  axum::serve(listener, build_router()).await.unwrap();
}`;

    expect(findProductionListenerConstructs(source)).toEqual(["axum::serve", "TcpListener::bind"]);
  });

  test("a test module cannot hide a production listener", () => {
    const source = `
async fn main() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
}

#[cfg(test)]
mod tests {
  async fn serve() { axum::serve(listener, app).await.unwrap(); }
}`;

    expect(findProductionListenerConstructs(source)).toEqual(["TcpListener::bind"]);
  });
});

describe("checkStateMountCoverage", () => {
  test("mount with a matching systems.toml identity is covered", () => {
    const { covered, uncovered } = checkStateMountCoverage(
      [{ tool: "mach-mono" }, { tool: "knowledge-base" }],
      new Set(["mach-mono", "knowledge-base", "backup-target"]),
    );
    expect(covered).toEqual(["mach-mono", "knowledge-base"]);
    expect(uncovered).toEqual([]);
  });

  test("mount with no systems.toml identity is uncovered — the real gap direction", () => {
    const { covered, uncovered } = checkStateMountCoverage(
      [{ tool: "mach-mono" }, { tool: "some-new-tool" }],
      new Set(["mach-mono"]),
    );
    expect(covered).toEqual(["mach-mono"]);
    expect(uncovered).toEqual(["some-new-tool"]);
  });

  test("empty mounts is trivially fully covered", () => {
    expect(checkStateMountCoverage([], new Set(["mach-mono"]))).toEqual({ covered: [], uncovered: [] });
  });
});

describe("extractSiblingRepoRefs", () => {
  test("plain top-level repo path", () => {
    expect(extractSiblingRepoRefs("see ~/Developer/mach-mono for the Swift monorepo")).toEqual(["mach-mono"]);
  });

  test("a direct sibling path planted in a public JSON example is detected", () => {
    const example = JSON.stringify({
      source: { path: "~/Developer/private-knowledge" },
    });
    expect(extractSiblingRepoRefs(example)).toEqual(["private-knowledge"]);
  });

  test("nested path resolves to the LAST segment, not the first — the planted regression case", () => {
    // This is the exact false positive the live run against Axon caught before the fix:
    // a naive first-segment match on ~/Developer/Personal/Knowledge-Base surfaced "Personal"
    // instead of "Knowledge-Base".
    expect(extractSiblingRepoRefs('"path": "~/Developer/Personal/Knowledge-Base"')).toEqual(["Knowledge-Base"]);
    expect(extractSiblingRepoRefs("~/Developer/Collab/VBB")).toEqual(["VBB"]);
  });

  test("$HOME form matches the same as ~", () => {
    expect(extractSiblingRepoRefs("$HOME/Developer/pi-agent")).toEqual(["pi-agent"]);
  });

  test("multiple distinct references in one blob", () => {
    expect(extractSiblingRepoRefs("~/Developer/Axon and ~/Developer/axon-overlay and $HOME/Developer/mach-mono")).toEqual([
      "Axon",
      "axon-overlay",
      "mach-mono",
    ]);
  });

  test("no match returns empty, not undefined/throw", () => {
    expect(extractSiblingRepoRefs("nothing relevant here, just prose")).toEqual([]);
  });

  test("a self root is skipped at any depth, not just at the top level", () => {
    // The live false positive: a path INSIDE the overlay reports its last segment, so a
    // basename-only self-check saw "config" and demanded a systems.toml entry for a repo
    // that has never existed. Depth is the fix — the root segment decides.
    expect(extractSiblingRepoRefs("see ~/Developer/example-overlay/config", ["example-overlay"])).toEqual([]);
    expect(extractSiblingRepoRefs("~/Developer/example-repo", ["example-repo"])).toEqual([]);
  });

  test("a self root does not swallow a genuine sibling in the same blob", () => {
    expect(
      extractSiblingRepoRefs("~/Developer/example-overlay/config and ~/Developer/mach-mono", ["example-overlay"]),
    ).toEqual(["mach-mono"]);
  });

  test("with no self roots declared, nothing is skipped", () => {
    expect(extractSiblingRepoRefs("~/Developer/example-overlay/config")).toEqual(["config"]);
  });

  test("does not match Developer/ paths outside $HOME/~ (narrow-by-design blind spot)", () => {
    expect(extractSiblingRepoRefs("/opt/Developer/something-else")).toEqual([]);
  });
});

describe("formatVersion", () => {
  test("tag-based describe shape", () => {
    expect(formatVersion("v1.2-3-gabc1234", "2026-07-16")).toBe("v1.2-3-gabc1234 (2026-07-16)");
  });

  test("bare-sha describe fallback (no tags in repo)", () => {
    expect(formatVersion("abc1234", "2026-07-16")).toBe("abc1234 (2026-07-16)");
  });

  test("git's own -dirty suffix passes through verbatim", () => {
    expect(formatVersion("abc1234-dirty", "2026-07-16")).toBe("abc1234-dirty (2026-07-16)");
  });

  test("missing commit date degrades to describe alone", () => {
    expect(formatVersion("abc1234", "")).toBe("abc1234");
  });

  test("empty describe reports honestly instead of ' ()'", () => {
    expect(formatVersion("", "2026-07-16")).toBe("(unknown — not a git checkout?)");
  });
});

describe("formatFetchAge", () => {
  const NOW = 1_800_000_000; // fixed epoch — the function is pure in (fetch, now)

  test("fresh fetch (<60s) is 'just now'", () => {
    expect(formatFetchAge(NOW - 5, NOW)).toBe("fetched just now");
  });

  test("minutes-old fetch", () => {
    expect(formatFetchAge(NOW - 25 * 60, NOW)).toBe("fetched 25 minute(s) ago");
  });

  test("hours-old fetch", () => {
    expect(formatFetchAge(NOW - 3 * 3600 - 40, NOW)).toBe("fetched 3 hour(s) ago");
  });

  test("days-old fetch", () => {
    expect(formatFetchAge(NOW - 2 * 86_400 - 3600, NOW)).toBe("fetched 2 day(s) ago");
  });

  test("missing FETCH_HEAD (null) reported honestly, not thrown", () => {
    expect(formatFetchAge(null, NOW)).toBe("no fetch recorded");
  });

  test("clock skew (fetch mtime in the future) clamps to 'just now' rather than negative", () => {
    expect(formatFetchAge(NOW + 120, NOW)).toBe("fetched just now");
  });
});

describe("findDecisionPathRot", () => {
  // The real 2026-07-16 failure: root-is-the-spine dissolved apps/, three entries kept
  // asserting it for twelve days, and nothing noticed.
  const present = (real: string[]) => (p: string) => real.includes(p);

  test("a named path that no longer exists is rot", () => {
    const rot = findDecisionPathRot(
      [{ slug: "stale", text: "consumes `apps/dashboard` over HTTP", assertsAbsent: [] }],
      present(["dashboard"]),
    );
    expect(rot).toEqual([{ slug: "stale", path: "apps/dashboard", kind: "missing" }]);
  });

  test("a path declared absent that came back is rot in the other direction", () => {
    const rot = findDecisionPathRot(
      [{ slug: "forbids", text: "no `tools/topology` binary", assertsAbsent: ["tools/topology"] }],
      present(["tools/topology"]),
    );
    expect(rot).toEqual([{ slug: "forbids", path: "tools/topology", kind: "present" }]);
  });

  test("asserts_absent silences the missing-path direction for the same path", () => {
    expect(findDecisionPathRot(
      [{ slug: "ok", text: "no `tools/topology` binary", assertsAbsent: ["tools/topology"] }],
      present([]),
    )).toEqual([]);
  });

  test("a crate-relative reference resolves against the passed bases", () => {
    expect(findDecisionPathRot(
      [{ slug: "ok", text: "an arm in `sources/mod.rs`", assertsAbsent: [] }],
      present(["capabilities/scouting/src/sources/mod.rs"]),
      ["", "capabilities/scouting/src/"],
    )).toEqual([]);
  });

  test("URLs, absolute paths, git refs and placeholders are not repo paths", () => {
    expect(findDecisionPathRot(
      [{ slug: "ok", text: "`https://a.com/b` `/usr/local/bin/x` `origin/main` `<vault>/Atlas` `~/Developer/x`", assertsAbsent: [] }],
      present([]),
    )).toEqual([]);
  });

  // The only false positive this check has produced: a model id backticked as the label of
  // its own huggingface link read as a repo path that had gone missing.
  test("a backticked slug labelling an external link is not a repo path", () => {
    expect(findDecisionPathRot(
      [{ slug: "ok", text: "[`org/model-name` at the audited commit](https://huggingface.co/org/model-name/tree/abc)", assertsAbsent: [] }],
      present([]),
    )).toEqual([]);
  });

  test("a real repo path outside a link label is still checked", () => {
    expect(findDecisionPathRot(
      [{ slug: "stale", text: "[docs](https://a.com/b) and `apps/dashboard`", assertsAbsent: [] }],
      present([]),
    )).toEqual([{ slug: "stale", path: "apps/dashboard", kind: "missing" }]);
  });
});

describe("collectWhyBlocks", () => {
  const doc = [
    "# punctuality", "", "Some prose naming `capabilities/other/thing.rs`.", "",
    "## Why this shape: Rust over a second engine", "",
    "It reads parquet from `src/aggregate.rs`.", "",
    "## Considered and declined", "", "Naming `nope/gone.rs` here must not be swept.", "",
  ].join("\n");

  test("captures only the why-block, not the surrounding README", () => {
    const blocks = collectWhyBlocks("capabilities/punctuality/README.md", doc);
    expect(blocks).toHaveLength(1);
    expect(blocks[0].text).toContain("src/aggregate.rs");
    expect(blocks[0].text).not.toContain("nope/gone.rs");
    expect(blocks[0].text).not.toContain("capabilities/other/thing.rs");
    expect(blocks[0].dir).toBe("capabilities/punctuality");
  });

  test("the heading topic lands in the slug so a finding is locatable", () => {
    expect(collectWhyBlocks("a/README.md", doc)[0].slug).toBe("a/README.md (Rust over a second engine)");
  });

  test("an asserts-absent comment inside the block is honoured", () => {
    const blocks = collectWhyBlocks("a/README.md",
      "## Why this shape: x\n\n<!-- asserts-absent: apps/dashboard, tools/topology -->\nno `apps/dashboard` here.\n");
    expect(blocks[0].assertsAbsent).toEqual(["apps/dashboard", "tools/topology"]);
  });

  test("a README with no why-block yields nothing", () => {
    expect(collectWhyBlocks("a/README.md", "# a\n\nplain prose with `some/path.rs`.\n")).toEqual([]);
  });

  test("two why-blocks in one file are captured separately", () => {
    const two = "## Why this shape: one\n\na\n\n## Why this shape: two\n\nb\n";
    expect(collectWhyBlocks("a/README.md", two)).toHaveLength(2);
  });
});

describe("parseEnvTemplateLines", () => {
  test("parses key/value pairs and strips inline comments", () => {
    const parsed = parseEnvTemplateLines(
      [
        "FOO=bar",
        "BAZ=\"quoted value\" # inline comment",
        "  # ignored comment",
        "",
        "X= # value can be blank",
      ].join("\n"),
    );
    expect(parsed).toEqual([
      { key: "FOO", value: "bar" },
      { key: "BAZ", value: "quoted value" },
      { key: "X", value: "" },
    ]);
  });
});

describe("findPlaintextSecretsInEnvTemplate", () => {
  test("ignores placeholders and flags obvious secret-like literals", () => {
    const leaks = findPlaintextSecretsInEnvTemplate(
      [
        "DOMAIN=example.local",
        "ADMIN_TOKEN=$argon2id$v=19$m=65536,t=3,p=4$...",
        "POSTGRES_PASSWORD=<required: private password>",
        "HA_TOKEN=<required: private token>",
        "DB_KEY=abc", // short, not enough entropy to be flagged
      ].join("\n"),
    );
    expect(leaks).toEqual(["ADMIN_TOKEN"]);
  });
});

describe("findDanglingDecisionRefs", () => {
  const alive = (s: string) => s === "root-is-the-spine-three-nouns";

  test("a mid-path segment is an HTTP route, not a repo citation", () => {
    // Measured 2026-09-05: the finance capability's route for recomputing
    // investment proposals failed this gate in four files at once, in Rust,
    // TypeScript and Markdown. The dissolved directory was at the repository
    // root, so a real citation always begins the path.
    expect(findDanglingDecisionRefs(
      [{ path: "capabilities/finance/src/server.rs", text: '"/api/decisions/run"' }], alive,
    )).toEqual([]);
    expect(findDanglingDecisionRefs(
      [{ path: "tools/demo-seed.ts", text: "post(`${base}/decisions/run`, {})" }], alive,
    )).toEqual([]);
    // And the narrowing must not swallow a real citation in the same file.
    expect(findDanglingDecisionRefs(
      [{ path: "README.md", text: "`/api/decisions/run` and `decisions/gone/README.md`" }], alive,
    )).toEqual([{ file: "README.md", slug: "gone" }]);
  });

  test("a citation with a directory in front of it is still a citation", () => {
    // The narrowing is on what a ROUTE looks like, not on a preceding slash. A
    // relative or prefixed path is the form a doc, a vault note or a generated
    // ARCHITECTURE.md line uses, and silencing it would leave the sweep blind to
    // the case it exists for.
    expect(findDanglingDecisionRefs(
      [{ path: "docs/guide.md", text: "See [x](./decisions/gone/README.md)." }], alive,
    )).toEqual([{ file: "docs/guide.md", slug: "gone" }]);
    expect(findDanglingDecisionRefs(
      [{ path: "ARCHITECTURE.md", text: "- `Knowledge-Base/decisions/gone/README.md`" }], alive,
    )).toEqual([{ file: "ARCHITECTURE.md", slug: "gone" }]);
    // An absolute URL is a route wherever its host came from.
    expect(findDanglingDecisionRefs(
      [{ path: "tools/x.ts", text: "http://127.0.0.1:8084/decisions/run" }], alive,
    )).toEqual([]);
  });

  test("a citation of a dissolved entry is reported", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "README.md", text: "See `decisions/dissolved-entry/README.md`." }], alive,
    )).toEqual([{ file: "README.md", slug: "dissolved-entry" }]);
  });

  test("benchmark datasets under benchmarks/decisions are not legacy decision records", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "research/local-decision-models.md", text: "`benchmarks/decisions/results/` and `research/benchmarks/decisions/cases.jsonl` and `[benchmarks/decisions/results/](benchmarks/decisions/results/)`" }],
      alive,
    )).toEqual([]);
  });

  test("a live entry is not reported", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "README.md", text: "See `CONTRIBUTING.md#three-architectural-nouns`." }], alive,
    )).toEqual([]);
  });

  test("the same dead slug twice in one file reports once", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "a.md", text: "decisions/gone and again decisions/gone/README.md" }], alive,
    )).toHaveLength(1);
  });

  test("catches it in a generator that emits the path rather than citing it", () => {
    expect(findDanglingDecisionRefs(
      [{ path: "tools/gen.sh", text: 'echo "See decisions/gone/README.md."' }], alive,
    )).toEqual([{ file: "tools/gen.sh", slug: "gone" }]);
  });
});

// The sweep's skip rule used to be a list of five file names, two of which described
// properties rather than exceptions (Axon#26). Repository and overlay names here are
// synthetic: a test that hardcodes this deployment's names would be the same mistake one
// level down.
describe("isSweepExempt", () => {
  const generated = "# Fixture Architecture\n\n> Auto-generated by tools/generate-fixture.sh. Do not edit manually.\n\n`~/Developer/example-repo/x`\n";

  test("a generated artifact is exempt by its own header, not by its name", () => {
    expect(isSweepExempt("FIXTURE.md", generated)).toBe(true);
  });

  test("the header only counts near the top, so a mention deep in prose is not an escape hatch", () => {
    const buried = `${"filler\n".repeat(20)}This file is auto-generated, honest.\n`;
    expect(isSweepExempt("notes.md", buried)).toBe(false);
  });

  test("a .example template is exempt — showing the path is its job", () => {
    expect(isSweepExempt("fixture.local.toml.example", 'overlay = "~/Developer/example-overlay"\n')).toBe(true);
  });

  test("the sanctioned indirection, the bootstrap namer and the sweep's own fixture stay exempt", () => {
    for (const f of ["tools/lib/paths.sh", "tools/install.sh", "tools/doctor.test.ts"]) {
      expect(isSweepExempt(f, '~/Developer/example-overlay')).toBe(true);
    }
  });

  test("an ordinary manifest is not exempt, whatever it holds", () => {
    expect(isSweepExempt("axon.toml", 'overlay = "~/Developer/example-overlay"\n')).toBe(false);
  });

  test("a file merely named like a template is not exempt", () => {
    // `.example` is a suffix rule, matching the env-template convention. A file called
    // example.md is documentation.
    expect(isSweepExempt("docs/example.md", "~/Developer/example-repo")).toBe(false);
  });
});

describe("whyBlockBases", () => {
  test("an owner, a unit inside it, and that unit's sources", () => {
    expect(whyBlockBases([
      "capabilities/example/README.md",
      "capabilities/example/src/lib.rs",
      "capabilities/example/src/sources/mod.rs",
    ])).toEqual([
      "",
      "capabilities/",
      "capabilities/example/",
      "capabilities/example/src/",
    ]);
  });

  test("a unit with no src/ gets no src/ base — the phantom the hand-list appended", () => {
    expect(whyBlockBases(["Packs/example/pack.toml", "Packs/example/skills/thing/SKILL.md"]))
      .not.toContain("Packs/example/src/");
  });

  test("every top-level owner the tree has, not the three someone remembered", () => {
    const bases = whyBlockBases([
      "dashboard/src/routes/+page.svelte",
      "schemas/service.toml.example",
      "tools/doctor.ts",
    ]);
    expect(bases).toContain("dashboard/");
    expect(bases).toContain("schemas/");
    expect(bases).toContain("tools/");
  });

  test("an untracked build tree cannot become a resolution base", () => {
    // node_modules/ is on disk beside a tracked package. Reading the directory instead of
    // the index would let a missing path resolve under a dependency and report clean.
    expect(whyBlockBases(["dashboard/package.json"])).toEqual(["", "dashboard/"]);
  });

  test("root-level files contribute the root base and nothing else", () => {
    expect(whyBlockBases(["README.md", "axon.toml"])).toEqual([""]);
  });
});

// systems.toml reachability (#18). The network half is a thin wrapper; everything that decides
// WHAT gets dialled and WHAT a failure means is pure, and that is what these hold. The failure
// mode being guarded is a check that reports green for an endpoint it never touched, so the
// skip reasons are asserted as carefully as the probe list.
describe("systems reachability probe targets", () => {
  test("a public inline http(s) url is probed as declared", () => {
    expect(resolveProbeTargets({ pub: { url: "https://example.test/health" } }, {}))
      .toEqual([{ id: "pub", url: "https://example.test/health", timeoutMs: PROBE_TIMEOUT_MS }]);
  });

  test("a private system resolves its url from the overlay, by the same id", () => {
    // The whole point of the sentinel: public Axon never holds the endpoint.
    expect(resolveProbeTargets(
      { priv: { url: "overlay:systems.local.toml" } },
      { priv: { url: "https://private.test" } },
    )).toEqual([{ id: "priv", url: "https://private.test", timeoutMs: PROBE_TIMEOUT_MS }]);
  });

  test("a private system with no overlay entry is skipped, not treated as the sentinel", () => {
    // Probing the literal string "overlay:systems.local.toml" would be a parse error at best and
    // a request to a resolver-invented host at worst.
    expect(resolveProbeTargets({ priv: { url: "overlay:systems.local.toml" } }, {}))
      .toEqual([{ id: "priv", skip: "private system, no url in the overlay" }]);
  });

  test("the overlay can opt an endpoint out, and Axon ships no opinion either way", () => {
    expect(resolveProbeTargets(
      { thing: { url: "https://example.test" } },
      { thing: { probe: "no", url: "https://example.test" } },
    )).toEqual([{ id: "thing", skip: 'overlay declares probe = "no"' }]);
  });

  test('url = "local" is a role, not an endpoint', () => {
    expect(resolveProbeTargets({ axon: { url: "local" } }, {}))
      .toEqual([{ id: "axon", skip: 'url = "local" — not an endpoint' }]);
  });

  test("a malformed url is skipped with a reason rather than thrown on", () => {
    expect(resolveProbeTargets({ broken: { url: "http://[not a url" } }, {}))
      .toEqual([{ id: "broken", skip: "url is not parseable" }]);
  });

  test("a non-http scheme is named, so ssh targets do not read as unreachable", () => {
    expect(resolveProbeTargets({ box: { url: "ssh://host.test" } }, {}))
      .toEqual([{ id: "box", skip: "ssh endpoint — only http(s) is probed" }]);
  });

  test("a credential-bearing url is refused, and the reason never carries the value", () => {
    const [target] = resolveProbeTargets({ leaky: { url: "https://user:pw@example.test" } }, {});
    expect(target).toEqual({ id: "leaky", skip: "url embeds credentials — not probed" });
    expect(JSON.stringify(target)).not.toContain("pw");
  });

  test("an entry with no url at all is skipped", () => {
    expect(resolveProbeTargets({ bare: { kind: "service" } }, {}))
      .toEqual([{ id: "bare", skip: "no url declared" }]);
  });

  test("the overlay can raise the timeout for one legitimately slow endpoint", () => {
    // The measured case: build.nvidia.com answers 202 in ~9.2s. Raising it for that entry beats
    // muting the check or slowing every probe down to the worst one.
    expect(resolveProbeTargets(
      { slow: { url: "https://slow.test" } },
      { slow: { probe_timeout_ms: 12000 } },
    )).toEqual([{ id: "slow", url: "https://slow.test", timeoutMs: 12000 }]);
  });

  test("a junk or non-positive timeout falls back rather than disabling the bound", () => {
    // An unbounded probe would hang the whole report on one bad declaration.
    for (const bad of ["soon", 0, -1, null]) {
      expect(resolveProbeTargets(
        { s: { url: "https://x.test" } },
        { s: { probe_timeout_ms: bad } },
      )).toEqual([{ id: "s", url: "https://x.test", timeoutMs: PROBE_TIMEOUT_MS }]);
    }
  });
});

describe("systems reachability failure classification", () => {
  test("an abort from the timeout signal is a timeout, never an outage", () => {
    expect(classifyProbeOutcome({ name: "TimeoutError" })).toBe("timeout");
    expect(classifyProbeOutcome({ name: "AbortError" })).toBe("timeout");
    expect(classifyProbeOutcome({ code: "ETIMEDOUT" })).toBe("timeout");
  });

  test("Bun's real refused-connection error shape is recognised, not just Node's", () => {
    // The exact object Bun 1.3.14 throws, captured from a live HEAD to 127.0.0.1:1 on 2026-08-06.
    // The first cut of this test asserted `{ code: "ECONNREFUSED" }` — a shape Bun never produces
    // — so it passed while every refused connection was really being reported as `unavailable`.
    // Live probing is what caught it; the fixture is now the captured reality.
    expect(classifyProbeOutcome({
      name: "Error",
      code: "ConnectionRefused",
      message: "Unable to connect. Is the computer able to access the url?",
    })).toBe("refused");
    expect(classifyProbeOutcome({ code: "ECONNREFUSED" })).toBe("refused");
  });

  test("anything else is unavailable rather than guessed at", () => {
    expect(classifyProbeOutcome(new Error("unable to verify the first certificate"))).toBe("unavailable");
    expect(classifyProbeOutcome(undefined)).toBe("unavailable");
  });

  test("a dead hostname is NOT classified here — that is the DNS step's answer", () => {
    // Bun reports ConnectionRefused for a nonexistent host too, so this function would call it
    // `refused`. That is correct as written and useless on its own, which is precisely why the
    // caller resolves DNS before it ever gets here. Asserted so the precondition cannot be
    // quietly dropped from the probe without a test going red.
    expect(classifyProbeOutcome({ code: "ConnectionRefused" })).toBe("refused");
  });
});

describe("backup receipts", () => {
  test("the receipt stamp backup.sh actually writes parses to its UTC epoch", () => {
    // The exact string tools/backup.sh emits — `date -u +%Y%m%dT%H%M%SZ` — copied from the live
    // overlay's finance receipt on 2026-09-08. A stamp invented here would only prove that this
    // parser agrees with itself.
    expect(parseReceiptTimestamp("20260906T210709Z")).toBe(Date.UTC(2026, 8, 6, 21, 7, 9) / 1000);
  });

  test("anything that is not that shape is no usable receipt, never a guess", () => {
    // Each of these would date a backup wrongly if it were coerced, and a wrongly dated backup
    // reports fresh. ISO-with-separators is the near miss worth pinning: it is what a second
    // writer would naturally emit, and it must be refused rather than half-read.
    for (const bad of ["2026-09-06T21:07:09Z", "20260906T210709", "20261306T210709Z", "", "never"]) {
      expect(parseReceiptTimestamp(bad)).toBeNull();
    }
  });

  test("the two thresholds mean different things, and never outranks both", () => {
    const day = 86_400;
    // capabilities/store's real contract: advise 1, stale 2.
    expect(backupAgeState(null, 1, 2)).toBe("never");
    expect(backupAgeState(2 * 3600, 1, 2)).toBe("ok");
    expect(backupAgeState(1.8 * day, 1, 2)).toBe("due");
    expect(backupAgeState(2.1 * day, 1, 2)).toBe("overdue");
    // A manifest that declares no cadence gets no invented one.
    expect(backupAgeState(400 * day, Number.NaN, Number.NaN)).toBe("unknown");
    // A zero threshold is a declaration, not an absence: `stale = 0` is the strictest contract
    // expressible, and `stale || default` would turn it into the loosest.
    expect(backupAgeState(60, 0, 0)).toBe("overdue");
  });

  test("a receipt whose archive is gone or short is a failure, not a fresh backup", () => {
    expect(classifyArchiveAtTarget({ exists: false, sizeBytes: null, flags: "", receiptBytes: 2361 }).level)
      .toBe("bad");
    expect(classifyArchiveAtTarget({ exists: true, sizeBytes: 12, flags: "", receiptBytes: 2361 }).level)
      .toBe("bad");
    expect(classifyArchiveAtTarget({ exists: true, sizeBytes: 2361, flags: "-", receiptBytes: 2361 }).level)
      .toBe("ok");
  });

  test("a failed attempt is reported even while the receipt still looks fresh", () => {
    // 2026-09-29: store's iCloud uploads had failed for days, two gated runs exited
    // non-zero, and the receipt-based age line still read "backed up 0.0d ago". The marker
    // the runner writes is what makes the failure outlive the log.
    const finding = attemptFinding(
      {
        exit_code: 1,
        at_epoch: 1_000_000,
        detail: "icloud-item: upload failed: Couldn't access your iCloud account",
      },
      1_000_000 + 3 * 3_600,
    );
    expect(finding.level).toBe("bad");
    expect(finding.detail).toContain("FAILED 3.0h ago");
    expect(finding.detail).toContain("exit 1");
    expect(finding.detail).toContain("iCloud account");
    // A marker with no reason still says the thing that matters.
    const terse = attemptFinding({ exit_code: 23, at_epoch: 1_000_000, detail: "" }, 1_000_000);
    expect(terse.detail).toContain("exit 23");
    expect(terse.detail.endsWith(")")).toBe(true);
  });

  test("an evicted archive is listed, named, correctly sized and not there", () => {
    // The live destination's own flag string on 2026-09-08, for capabilities/store's archive.
    // This is the case the shipped detector could not see: every other signal about it is right.
    const verdict = classifyArchiveAtTarget({
      exists: true,
      sizeBytes: 39_973_563,
      flags: "compressed,dataless",
      receiptBytes: 39_973_563,
    });
    expect(verdict.level).toBe("warn");
    expect(verdict.detail).toContain("offloaded");
    expect(verdict.detail).toContain("cannot prove the cloud copy");
    const misleadingSize = classifyArchiveAtTarget({
      exists: true, sizeBytes: 0, flags: "dataless", receiptBytes: 39_973_563,
    });
    expect(misleadingSize.level).toBe("warn");
    expect(misleadingSize.detail).toContain("size differs");
    // `compressed` on its own is ordinary APFS compression and says nothing about eviction.
    expect(classifyArchiveAtTarget({ exists: true, sizeBytes: 10, flags: "compressed", receiptBytes: 10 }).level)
      .toBe("ok");
  });
});

describe("scheduled producers", () => {
  test("launchctl's real table is parsed, header and dashes and all", () => {
    // Captured verbatim from `launchctl list` on 2026-09-08. The dash columns are the shapes a
    // hand-written fixture would omit: a scheduled job is not running most of the time, so its PID
    // is always `-`, and `com.axon.backup` carries the last exit status that matters here.
    const jobs = parseLaunchdJobs(
      [
        "PID\tStatus\tLabel",
        "-\t0\tcom.axon.sparpreis-watch",
        "787\t0\tcom.axon.sjel-status",
        "-\t1\tcom.axon.backup",
      ].join("\n"),
    );
    expect(jobs.size).toBe(3);
    expect(jobs.get("com.axon.backup")).toEqual({ pid: null, lastExit: 1 });
    expect(jobs.get("com.axon.sjel-status")).toEqual({ pid: 787, lastExit: 0 });
    // The header must not become a job. It would make `loaded` true for a label called "Label",
    // which is harmless — and it would also make the table's size a lie in any count derived here.
    expect(jobs.has("Label")).toBe(false);
    // Absent, which is how "launchd does not have this unit" is spelled.
    expect(jobs.get("com.axon.host-patch")).toBeUndefined();
  });

  test("the unit's own interval and log paths are read out of the plist", () => {
    // The shape tools/templates/launchd-schedule.plist.tmpl renders, so the parser is pinned to
    // the file service-runner.sh actually writes rather than to a plist invented here.
    const plist = `<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.axon.feed-sweep</string>
  <key>RunAtLoad</key>
  <true/>
  <key>StartInterval</key>
  <integer>21600</integer>
  <key>StandardOutPath</key>
  <string>/tmp/axon-feed-sweep-schedule.log</string>
  <key>StandardErrorPath</key>
  <string>/tmp/axon-feed-sweep-schedule.err</string>
</dict>
</plist>`;
    expect(parseLaunchdSchedule(plist)).toEqual({
      intervalSeconds: 21600,
      stdoutPath: "/tmp/axon-feed-sweep-schedule.log",
      stderrPath: "/tmp/axon-feed-sweep-schedule.err",
    });
    // A watchdog unit has no StartInterval. Reading one out of it would invent a cadence.
    expect(parseLaunchdSchedule("<dict><key>KeepAlive</key><true/></dict>").intervalSeconds).toBeNull();
  });

  const producer = (over: Partial<Parameters<typeof classifyScheduledProducer>[0]>) =>
    classifyScheduledProducer({
      name: "feed-sweep",
      unitInstalled: true,
      loaded: true,
      lastExit: 0,
      intervalSeconds: 21600,
      lastOutputAgeSeconds: 600,
      ...over,
    });

  test("a producer that ran inside its interval is the only ok answer", () => {
    expect(producer({}).level).toBe("ok");
    expect(producer({ lastOutputAgeSeconds: 21_599 }).level).toBe("ok");
  });

  test("one interval late is a warning, three is a fault", () => {
    // Three, not two, because launchd's StartInterval does not fire while the machine sleeps and
    // fires once on wake — so a closed lid legitimately costs an hourly job two intervals.
    expect(producer({ lastOutputAgeSeconds: 21_600 }).level).toBe("warn");
    expect(producer({ lastOutputAgeSeconds: 64_799 }).level).toBe("warn");
    expect(producer({ lastOutputAgeSeconds: 64_800 }).level).toBe("bad");
    expect(producer({ lastOutputAgeSeconds: 64_800 }).message).toContain("missed at least two runs");
  });

  test("a failed run outranks a healthy age, because a fast failure still touches the log", () => {
    const v = producer({ lastExit: 1, lastOutputAgeSeconds: 5 });
    expect(v.level).toBe("bad");
    expect(v.message).toContain("exited 1");
  });

  test("an unloaded unit is a timer that cannot fire, whatever its logs say", () => {
    // The state the orchestrator left com.axon.host-patch in on 2026-09-08. Its log is recent
    // because it ran before it was unloaded, so age alone reports it perfectly healthy.
    const v = producer({ loaded: false, lastOutputAgeSeconds: 60 });
    expect(v.level).toBe("warn");
    expect(v.message).toContain("launchd has not loaded it");
  });

  test("no output on record is not the claim that it never ran", () => {
    const v = producer({ lastOutputAgeSeconds: null });
    expect(v.level).toBe("warn");
    expect(v.message).toContain("no output this machine still holds");
  });

  test("a missing unit is named here and judged by the persistence check", () => {
    // finance-prices' real state: a manifest declaring `schedule = "24h"` and no installed unit.
    // Counting it as a fault here too would print one condition as two problems.
    const v = producer({ name: "finance-prices", unitInstalled: false, loaded: false });
    expect(v.level).toBe("ok");
    expect(v.message).toContain("no unit installed");
  });

  test("ages read as the unit a person would use", () => {
    expect(formatAge(9)).toBe("9s");
    expect(formatAge(600)).toBe("10m");
    expect(formatAge(21_600)).toBe("6.0h");
    expect(formatAge(345_600)).toBe("4.0d");
  });
});
