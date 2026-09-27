// tools/lib/release.ts — which git tags count as release tags, for the TypeScript callers.
//
// Why this exists: `git describe --tags` answers with ANY reachable tag. An archive marker, a
// vendor tag or a nightly then silently becomes the reported version of a checkout, and every
// "installed vs latest" readout inherits that lie. Restricting the match to the release line is
// the fix; keeping the pattern in one place is what stops it drifting back.
//
// The pattern itself lives in axon.toml `[release] tag_glob`, not in this file. tools/update.sh
// and tools/lib/delta.sh ask the same question from bash, and tools/doctor and tools/self exec
// bun directly without sourcing a shell library — so a tracked manifest key is the only home both
// sides can actually read. See CONTRIBUTING.md#the-release-line.

import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * The `[release] tag_glob` value from `<axonRoot>/axon.toml`.
 *
 * Throws when the key is missing rather than falling back to a literal: a default here would put
 * the pattern in two places again, which is the defect this module removes.
 */
export function releaseTagGlob(axonRoot: string): string {
  const manifest = join(axonRoot, "axon.toml");
  const parsed = Bun.TOML.parse(readFileSync(manifest, "utf8")) as {
    release?: { tag_glob?: unknown };
  };
  const glob = parsed.release?.tag_glob;
  if (typeof glob !== "string" || glob.length === 0) {
    throw new Error(`release.ts: no [release] tag_glob in ${manifest}`);
  }
  return glob;
}
