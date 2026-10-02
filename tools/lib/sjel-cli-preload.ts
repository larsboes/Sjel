// tools/lib/sjel-cli-preload.ts — build the sjel-cli binary once, before any test runs.
//
// Several suites call tools/capability.sh at collection time (tools/lib/demo-endpoints.test.ts
// through registry()), and that launcher builds sjel-cli when the binary is missing or stale.
// On a cold checkout, CI included, every such call started the same release build at once and
// waited on cargo's lock, and the calls failed mid-build (CI run 37009234105, 2026-10-02). One
// build here, before collection, leaves every later call a ~20 ms exec. When the binary is
// already fresh this costs one stat walk.
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const root = join(import.meta.dir, "..", "..");
execFileSync(join(root, "tools/capability.sh"), ["-h"], { stdio: ["ignore", "ignore", "inherit"] });
