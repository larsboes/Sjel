import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { Plugin } from "vite";

const ID = "virtual:sjel-research";
const RESOLVED = `\0${ID}`;

/**
 * research/*.md, the two project registers and the benchmark runs, bundled into the /research routes.
 *
 * Read at build time, not fetched: the files are tracked source, the same on every machine,
 * and the published demo has no backend to ask. Only the research routes import the module,
 * so the text lands in their lazy chunk and not in the eager bundle bundleGuard() measures.
 */
export function research(root: string): Plugin {
  return {
    name: "sjel-research",
    resolveId: (id) => (id === ID ? RESOLVED : undefined),
    load(id) {
      if (id !== RESOLVED) return undefined;
      const dir = join(root, "research");
      const entries = readdirSync(dir)
        .filter((file) => file.endsWith(".md"))
        .sort()
        .map((file) => {
          const path = join(dir, file);
          this.addWatchFile(path);
          return { file, markdown: readFileSync(path, "utf8") };
        });
      for (const file of ["upstreams.toml", "systems.toml"]) this.addWatchFile(join(root, file));
      // research/benchmarks/<suite>/results/*.json, one file per run. The suite directory and its
      // README are the method; the result files are what a run measured, left as written.
      const benchDir = join(dir, "benchmarks");
      const runs = existsSync(benchDir)
        ? readdirSync(benchDir, { withFileTypes: true })
            .filter((d) => d.isDirectory())
            .flatMap((suite) => {
              const resultsDir = join(benchDir, suite.name, "results");
              if (!existsSync(resultsDir)) return [];
              return readdirSync(resultsDir)
                .filter((file) => file.endsWith(".json"))
                .sort()
                .map((file) => {
                  const path = join(resultsDir, file);
                  this.addWatchFile(path);
                  return { suite: suite.name, file, result: JSON.parse(readFileSync(path, "utf8")) };
                });
            })
        : [];
      const registers = execFileSync("bun", [join(root, "tools/research-registers.ts")], {
        encoding: "utf8",
        timeout: 15_000,
      });
      return [
        `export const entries = ${JSON.stringify(entries)};`,
        `export const registers = ${registers.trim()};`,
        `export const benchmarks = ${JSON.stringify(runs)};`,
        "",
      ].join("\n");
    },
  };
}
