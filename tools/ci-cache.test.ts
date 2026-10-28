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
