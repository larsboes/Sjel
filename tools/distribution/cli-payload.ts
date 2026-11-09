import { createHash } from "node:crypto";
import { chmodSync, constants, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { completeJavaScriptNotices, type RustSource, writeRustNotices } from "./cli-notices.ts";

export const cliBinaries = ["sjel-cli", "sjel-claude-config", "sjel-mcp"];
const helpers = ["harnesses", "packs.sh", "packs-claude", "packs-codex", "packs-copilot", "packs-opencode", "packs-pi", "updates", "audit", "agent-integrations.sh", "host-patch.sh", "skills", "pack-drift-hook", "copilot", "inference-keys", "materialize-inference-key", "local-build", "sjel-context", "self", "ignore-policy", "protection-zones", "toolchain-check", "doctor", "model-check"];
const installers = ["tools/distribution/cli-install.sh", "tools/distribution/cli-management.sh"];
const metadata = ["sjel", "LICENSE", "README.md", "CONTRIBUTING.md", "ARCHITECTURE.md", "AGENTS.md", "upstreams.toml", "toolchain.toml", "self.json"];
const runtimePackages = ["harness/pi-packages/pi-subagents", "harness/pi-packages/pi-web-access"];

type Profile = { name: string; description?: string; packs: string[]; except?: string[]; skills?: Record<string, string[]> };
type Profiles = { profile: Profile[]; unit_hold?: Record<string, string[]>; harness_profile?: Record<string, string> };
export interface CliSelection { paths: string[]; profiles: Profiles; profile: Profile; packs: Map<string, string[]> }

function tracked(root: string): string[] {
  const result = Bun.spawnSync(["git", "-c", "core.fsmonitor=false", "ls-files", "-z"], { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (result.exitCode) throw new Error("cannot enumerate tracked CLI payload");
  return result.stdout.toString().split("\0").filter(Boolean).sort();
}

export function regularFile(root: string, path: string) {
  if (/[\r\n\\]/.test(path) || path.split("/").some((part) => !part || part === "." || part === "..")) throw new Error(`unsafe payload path: ${path}`);
  if (lstatSync(root).isSymbolicLink()) throw new Error(`CLI payload root is a symlink: ${root}`);
  const parts = path.split("/");
  for (let i = 1; i <= parts.length; i++) {
    const entry = lstatSync(join(root, ...parts.slice(0, i)));
    if (entry.isSymbolicLink()) throw new Error(`CLI payload contains a symlink: ${path}`);
    if (i === parts.length ? !entry.isFile() : !entry.isDirectory()) throw new Error(`not a regular payload file: ${path}`);
  }
}

function held(unit: string, holds: string[]) {
  return holds.some((hold) => hold.endsWith("/") ? unit.startsWith(hold) : unit === hold || unit.startsWith(hold + "/"));
}

export function selectCliSources(root: string): CliSelection {
  regularFile(root, "profiles.toml");
  const profiles = Bun.TOML.parse(readFileSync(join(root, "profiles.toml"), "utf8")) as unknown as Profiles;
  const profile = profiles.profile.find((entry) => entry.name === "coding-harness");
  if (!profile) throw new Error("profiles.toml needs coding-harness");
  const all = tracked(root);
  const trackedSet = new Set(all);
  const globalHolds = profiles.unit_hold?.["*"] ?? [];
  const piHolds = profiles.unit_hold?.pi ?? [];
  const available = all.filter((path) => /^Packs\/[^/]+\/pack.toml$/.test(path)).map((path) => path.split("/")[1]);
  const selected = profile.packs.includes("*") ? available : profile.packs;
  const packs = new Map<string, string[]>();
  for (const name of selected.filter((pack) => !profile.except?.includes(pack))) {
    if (!/^[a-z0-9-]+$/.test(name) || held(`${name}/`, globalHolds)) continue;
    const manifest = `Packs/${name}/pack.toml`;
    if (!trackedSet.has(manifest)) throw new Error(`untracked or missing Pack manifest: ${manifest}`);
    regularFile(root, manifest);
    const declared = Bun.TOML.parse(readFileSync(join(root, manifest), "utf8")).skills as string[];
    const skills = profile.skills?.[name] ?? declared;
    if (!Array.isArray(skills) || skills.some((skill) => !declared.includes(skill) || !/^[a-z0-9-]+$/.test(skill))) throw new Error(`invalid coding-harness skill subset: ${name}`);
    packs.set(name, skills.filter((skill) => !held(`${name}/${skill}`, globalHolds)));
  }
  const required = [...metadata, ...helpers.map((name) => `tools/${name}`), ...installers];
  for (const path of required) if (!trackedSet.has(path)) throw new Error(`required CLI source is not tracked: ${path}`);
  const paths = all.filter((path) => {
    if (required.includes(path)) return true;
    if (/^tools\/lib\//.test(path)) return !/\.test\.|test-support/.test(path);
    if (/^tools\/templates\/(claude-code|codex|copilot|pi)\//.test(path)) return true;
    if (path.startsWith("schemas/")) return true;
    if (!path.startsWith("Packs/")) return false;
    const [, pack, category, unit] = path.split("/");
    if (!packs.has(pack)) return false;
    const local = path.slice("Packs/".length);
    const normalized = category === "skills" ? `${pack}/${unit}${path.slice(`Packs/${pack}/skills/${unit}`.length)}` : local;
    if (held(normalized, globalHolds) || held(local, globalHolds)) return false;
    if (["pi-packages", "extensions", "agents"].includes(category) && held(local, piHolds)) return false;
    if (category === "skills" && !packs.get(pack)!.includes(unit)) return false;
    return true;
  });
  // Validate before building or copying; neither directory symlinks nor ignored files are admitted.
  for (const path of paths) regularFile(root, path);
  return { paths, profiles, profile, packs };
}

function put(stage: string, path: string, text: string, executable = false) {
  mkdirSync(dirname(join(stage, path)), { recursive: true });
  writeFileSync(join(stage, path), text);
  if (executable) chmodSync(join(stage, path), 0o755);
}

export function payloadFiles(stage: string, prefix = ""): string[] {
  return readdirSync(join(stage, prefix), { withFileTypes: true }).flatMap((entry) => {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isSymbolicLink()) throw new Error(`CLI payload contains a symlink: ${path}`);
    if (/[\r\n\\]/.test(path)) throw new Error(`unsafe payload path: ${path}`);
    if (entry.isDirectory()) return payloadFiles(stage, path);
    if (!entry.isFile()) throw new Error(`not a regular payload file: ${path}`);
    return [path];
  }).sort();
}

export function detachHardlinks(stage: string) {
  for (const path of payloadFiles(stage)) {
    const file = join(stage, path);
    if (lstatSync(file).nlink <= 1) continue;
    const temporary = `${file}.sjel-detached`;
    copyFileSync(file, temporary, constants.COPYFILE_EXCL);
    renameSync(temporary, file);
  }
}

function profilesText(selection: CliSelection, disabled: string[]) {
  const { profiles, profile, packs } = selection;
  const lines = ["[unit_hold]"];
  for (const [key, value] of Object.entries({ ...profiles.unit_hold, pi: [...(profiles.unit_hold?.pi ?? []), ...disabled] })) lines.push(`${JSON.stringify(key)} = ${JSON.stringify(value)}`);
  lines.push("", "[harness_profile]");
  for (const harness of new Set(["*", "claude", "codex", "pi", "copilot", "opencode", "antigravity", ...Object.keys(profiles.harness_profile ?? {})])) lines.push(`${JSON.stringify(harness)} = "coding-harness"`);
  lines.push("", "[[profile]]", 'name = "coding-harness"', `description = ${JSON.stringify(profile.description ?? "Coding harness tooling")}`, `packs = ${JSON.stringify([...packs.keys()])}`, "", "[profile.skills]");
  for (const [pack, skills] of packs) lines.push(`${JSON.stringify(pack)} = ${JSON.stringify(skills)}`);
  return lines.join("\n") + "\n";
}

/** Render a pipe-safe POSIX entry point with the release version as its default.
 * @throws If the version or embedded script cannot be represented safely.
 */
export function standaloneInstaller(version: string, installer: string): string {
  const delimiter = "SJEL_EMBEDDED_BASH_INSTALLER";
  if (!/^v\d+\.\d+\.\d+$/.test(version) || installer.split(/\r?\n/).includes(delimiter)) throw new Error("invalid standalone installer input");
  return `#!/bin/sh
set -eu
main() {
  command -v bash >/dev/null 2>&1 || { printf '%s\\n' 'sjel install: Bash is required on macOS and Linux' >&2; return 1; }
  bash -s -- --version '${version}' "$@" <<'${delimiter}'
${installer.trimEnd()}
${delimiter}
}
main "$@"
`;
}

function prebuiltWrapper(binary: string) {
  return `#!/bin/bash\nset -eu\n_here="$(cd "$(dirname "\${BASH_SOURCE[0]:-$0}")" && pwd)"\n. "$_here/../lib/paths.sh"\nBIN="$SJEL_ROOT/target/release/${binary}"\n[ -x "$BIN" ] || { echo "${binary}: packaged binary missing; repair the CLI installation" >&2; exit 2; }\nexec "$BIN" "$@"\n`;
}

export function verifyLocalImports(stage: string) {
  for (const path of payloadFiles(stage).filter((path) => /\.[cm]?[jt]sx?$/.test(path) && !/\/test[s]?\/|\.test\./.test(path))) {
    const source = readFileSync(join(stage, path), "utf8");
    for (const match of source.matchAll(/(?:\bfrom\s*|\bimport\s*\(\s*|\brequire\s*\(\s*)["'](\.[^"']+)["']/g)) {
      const base = resolve(stage, dirname(path), match[1]);
      if (!base.startsWith(resolve(stage) + sep)) throw new Error(`local import escapes payload: ${path}: ${match[1]}`);
      const candidates = [base, `${base}.ts`, `${base}.js`, join(base, "index.ts"), base.replace(/\.js$/, ".ts")];
      if (!candidates.some((candidate) => existsSync(candidate))) throw new Error(`missing local payload import: ${path}: ${match[1]}`);
    }
  }
}

export function writeCliPayload(root: string, stage: string, target: string, version: string, platform: string, build: boolean, selection = selectCliSources(root), sources: RustSource[] = []) {
  for (const binary of cliBinaries) {
    regularFile(target, `release/${binary}`);
    if (!(lstatSync(join(target, "release", binary)).mode & 0o111)) throw new Error(`CLI binary is not executable: ${binary}`);
  }
  mkdirSync(stage, { recursive: true });
  const disabled = build ? [] : runtimePackages;
  for (const path of selection.paths) {
    if (disabled.some((unit) => path.startsWith(`Packs/${unit}/`))) continue;
    regularFile(root, path);
    mkdirSync(dirname(join(stage, path)), { recursive: true });
    copyFileSync(join(root, path), join(stage, path));
  }
  for (const binary of cliBinaries) {
    const path = `target/release/${binary}`;
    mkdirSync(dirname(join(stage, path)), { recursive: true });
    copyFileSync(join(target, "release", binary), join(stage, path));
    chmodSync(join(stage, path), 0o755);
  }
  put(stage, "profiles.toml", profilesText(selection, disabled));
  for (const [pack, skills] of selection.packs) {
    const path = `Packs/${pack}/pack.toml`;
    const text = readFileSync(join(stage, path), "utf8");
    if (!/^skills\s*=\s*\[[^\]]*\]/m.test(text)) throw new Error(`cannot narrow Pack skills: ${pack}`);
    put(stage, path, text.replace(/^skills\s*=\s*\[[^\]]*\]/m, `skills = ${JSON.stringify(skills)}`));
  }
  put(stage, "sjel.toml", 'overlay = "~/.local/share/sjel/overlay"\n');
  put(stage, "tools/claude-code-config/claude-code-config", prebuiltWrapper("sjel-claude-config"), true);
  put(stage, "tools/sjel-mcp/sjel-mcp", prebuiltWrapper("sjel-mcp"), true);
  verifyLocalImports(stage);
  const prepared: string[] = [];
  if (build) {
    for (const unit of runtimePackages) {
      const dir = join(stage, "Packs", unit);
      if (!existsSync(dir)) continue;
      for (const name of ["package.json", "bun.lock", "bunfig.toml"]) regularFile(dir, name);
      const result = Bun.spawnSync(["bun", "install", "--frozen-lockfile", "--production", "--omit=peer", "--ignore-scripts", "--linker=hoisted"], { cwd: dir, stdout: "inherit", stderr: "inherit" });
      if (result.exitCode) throw new Error(`production dependencies failed: ${unit}`);
      // Package executables are not used by these extensions; Bun's .bin links are not payload files.
      rmSync(join(dir, "node_modules/.bin"), { recursive: true, force: true });
      payloadFiles(dir);
      prepared.push(unit);
    }
  }
  completeJavaScriptNotices(prepared.map((unit) => join(stage, "Packs", unit)));
  // Bun's Linux cache hardlinks must not become archive link entries.
  detachHardlinks(stage);
  put(stage, "CLI_RUNTIME.json", JSON.stringify({ schema: 1, pi_dependencies: build ? "prepared" : "unprepared", prepared, disabled, reason: build ? "Frozen production dependencies; lifecycle scripts disabled" : "No-build fixture: dependency-backed Pi packages omitted and held" }, null, 2) + "\n");
  put(stage, "distribution.json", JSON.stringify({ schema: 1, kind: "cli", version, platform, source_builds: false, core: [] }, null, 2) + "\n");
  put(stage, "CLI_VERSION", version + "\n");
  put(stage, "CLI_PLATFORM", platform + "\n");
  if (sources.length) writeRustNotices(stage, sources, version, platform);
  const sums = payloadFiles(stage).map((path) => `${createHash("sha256").update(readFileSync(join(stage, path))).digest("hex")}  ${path}\n`).join("");
  put(stage, "PAYLOAD_SHA256SUMS", sums);
  return sources;
}
