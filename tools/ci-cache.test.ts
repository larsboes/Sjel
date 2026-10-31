import { expect, test } from "bun:test";
import { join } from "node:path";

test("Pages restores Rust dependencies but saves only on main", async () => {
  const text = await Bun.file(join(import.meta.dir, "../.github/workflows/pages.yml")).text();
  const workflow = Bun.YAML.parse(text) as any;
  const steps = workflow.jobs.build.steps;
  const caches = steps.filter((step: any) => typeof step.uses === "string" && /(?:actions\/cache|Swatinem\/rust-cache)@/.test(step.uses));
  expect(caches).toHaveLength(1);
  expect(caches[0].uses).toStartWith("Swatinem/rust-cache@");
  expect(caches[0].with["save-if"]).toBe("${{ github.ref == 'refs/heads/main' }}");
});

test("ci-cache marks an older same-prefix entry and a closed pull request's entry as dead", async () => {
  const { dead } = await import("./ci-cache.ts");
  const c = (id: number, key: string, ref: string, last_accessed_at: string) => ({ id, key, ref, size_in_bytes: 1, last_accessed_at });
  const caches = [
    c(1, "v0-rust-build-Linux-x64-b9e20461-b9cac53a", "refs/heads/main", "2026-10-28"),
    c(2, "v0-rust-build-Linux-x64-770ca156-b9cac53a", "refs/heads/main", "2026-10-30"),
    c(3, "v0-rust-build-Linux-x64-770ca156-b9cac53a", "refs/pull/7/merge", "2026-10-29"),
    c(4, "v0-rust-build-Linux-x64-770ca156-b9cac53a", "refs/pull/8/merge", "2026-10-29"),
    c(5, "v0-rust-quality-Linux-x64-770ca156", "refs/heads/main", "2026-10-01"),
    // Created after 6 but never restored since: the live entry is the one read last.
    c(6, "v0-rust-tests-Linux-x64-770ca156-b9cac53a", "refs/heads/main", "2026-10-31"),
    c(7, "v0-rust-tests-Linux-x64-b9e20461-b9cac53a", "refs/heads/main", "2026-10-28"),
  ];
  expect([...dead(caches, new Set([8]))].sort()).toEqual([[1, "superseded"], [3, "closed-pr"], [7, "superseded"]]);
});
