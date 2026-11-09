#!/usr/bin/env bun
// Load the installed profile with an independently installed Pi SDK; never prompt a model.
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, realpathSync } from "node:fs";
import { join, resolve, sep } from "node:path";

const [installation, piPackage] = process.argv.slice(2);
assert(installation && piPackage, "usage: cli-harness-smoke.ts INSTALLED_ROOT PI_PACKAGE");
assert.equal(process.env.SJEL_CLI_SMOKE, "1", "run only in the isolated CLI smoke environment");
const home = realpathSync(process.env.HOME!);
const root = realpathSync(installation);
assert(root.startsWith(home + sep), "installation must belong to the isolated HOME");
const agentDir = join(home, ".pi", "agent");
const cwd = join(home, "harness-workspace");
mkdirSync(cwd, { recursive: true });
const settings = JSON.parse(readFileSync(join(agentDir, "settings.json"), "utf8"));
const configured = [...settings.extensions];
for (const directory of settings.packages) {
  const manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
  for (const entry of manifest.pi.extensions) configured.push(resolve(directory, entry));
}
assert(configured.length > 0, "installed profile has no extensions");
for (const path of configured) assert(realpathSync(path).startsWith(root + sep), "extension escapes installed payload");
const pi = await import(join(resolve(piPackage), "dist", "index.js"));
const settingsManager = pi.SettingsManager.create(cwd, agentDir);
settingsManager.applyOverrides({ enableAnalytics: false, enableInstallTelemetry: false, cacheWarming: "off" });
const loader = new pi.DefaultResourceLoader({ cwd, agentDir, settingsManager, noContextFiles: true });
await loader.reload();
const loaded = loader.getExtensions();
assert.deepEqual(loaded.errors, [], "Pi extension loading failed");
const paths = loaded.extensions.map((extension: { resolvedPath: string }) => realpathSync(extension.resolvedPath));
assert.deepEqual(new Set(paths), new Set(configured.map((path: string) => realpathSync(path))), "Pi did not load exactly the installed profile");
const modelRuntime = await pi.ModelRuntime.create({
  authPath: join(agentDir, "auth.json"), modelsPath: join(agentDir, "models.json"),
  allowModelNetwork: false, refreshOnCreate: false,
});
const { session } = await pi.createAgentSession({
  cwd, agentDir, modelRuntime, settingsManager, resourceLoader: loader,
  sessionManager: pi.SessionManager.inMemory(cwd),
});
const errors: unknown[] = [];
try {
  await session.bindExtensions({ onError: (error: unknown) => errors.push(error) });
  assert.deepEqual(errors, [], "Pi extension startup failed");
  assert(session.getActiveToolNames().includes("Agent"), "packaged subagent tool is missing");
  assert(session.getActiveToolNames().includes("web_search"), "packaged web search tool is missing");
  console.log(`Pi ${JSON.parse(readFileSync(join(piPackage, "package.json"), "utf8")).version}: loaded ${paths.length} installed extensions; session startup and packaged tools passed without a model request.`);
} finally {
  session.dispose();
}
