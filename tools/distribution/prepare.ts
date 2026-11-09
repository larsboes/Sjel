#!/usr/bin/env bun
// Release orchestration uses the same Bun build runtime as the website build.
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { cliBinaries, selectCliSources, standaloneInstaller, writeCliPayload } from "./cli-payload.ts";
import { compiledRustSources, writeCorrespondingSource } from "./cli-notices.ts";

const root = resolve(import.meta.dir, "../..");
const core = ["sjel-status", "devices", "entities", "store"];
const binaries = ["sjel-cli", "sjel-status", "devices-server", "entities-server", "sjel-storage", "sjel-capability-auth"];
const platform = `${process.arch === "arm64" ? "aarch64" : process.arch === "x64" ? "x86_64" : process.arch}-${process.platform === "darwin" ? "apple-darwin" : "unknown-linux-gnu"}`;

function minimumMacOS(): string {
  const manifest = Bun.TOML.parse(readFileSync(join(root, "apps/gpui/Cargo.toml"), "utf8"));
  const version = manifest.package?.metadata?.sjel?.minimum_macos;
  if (typeof version !== "string" || !/^\d+\.\d+$/.test(version)) throw new Error("GPUI manifest needs a minimum macOS version");
  return version;
}

function run(argv: string[], cwd = root) {
  const result = Bun.spawnSync(argv, { cwd, stdout: "inherit", stderr: "inherit" });
  if (result.exitCode !== 0) throw new Error(`${argv[0]} failed (${result.exitCode})`);
}

function sha(path: string) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function copy(from: string, to: string) {
  mkdirSync(dirname(to), { recursive: true });
  cpSync(from, to, { recursive: true, dereference: false, verbatimSymlinks: true });
}

function copyPublicTree(relative: string, destination: string) {
  const list = Bun.spawnSync(["git", "-c", "core.fsmonitor=false", "ls-files", "-z", "--", relative], { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (list.exitCode !== 0) throw new Error(`cannot enumerate tracked ${relative}`);
  const paths = list.stdout.toString().split("\0").filter(Boolean);
  if (!paths.length) throw new Error(`no tracked files in ${relative}`);
  for (const path of paths) copy(join(root, path), join(destination, path.slice(relative.length + 1)));
}

function files(dir: string, prefix = ""): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const name = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isSymbolicLink()) throw new Error(`release payload contains a symlink: ${name}`);
    return entry.isDirectory() ? files(join(dir, entry.name), name) : [name];
  }).sort();
}

// Source manifests: same order as `sjel_manifest_for` in tools/lib/paths.sh.
function unitDir(name: string) {
  const dir = ["crates", "services", "capabilities"].map((parent) => join(root, parent, name))
    .find((candidate) => existsSync(join(candidate, "service.toml")));
  if (!dir) throw new Error(`${name}: no service.toml under crates/, services/ or capabilities/`);
  return dir;
}

function service(name: string) {
  return Bun.TOML.parse(readFileSync(join(unitDir(name), "service.toml"), "utf8")) as Record<string, unknown>;
}

function feature(name: string) {
  const path = join(unitDir(name), "feature.toml");
  if (!existsSync(path)) throw new Error(`${name}: a feature.toml is required for a catalogued package`);
  const metadata = Bun.TOML.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
  const allowed = new Set(["schema", "title", "summary", "category", "recommend_topics", "data_access"]);
  if (metadata.schema !== 1 || typeof metadata.title !== "string" || !metadata.title.trim()
    || metadata.title.length > 80
    || typeof metadata.summary !== "string" || !metadata.summary.trim() || metadata.summary.length > 240
    || typeof metadata.category !== "string" || !metadata.category.trim() || metadata.category.length > 40
    || Object.keys(metadata).some((key) => !allowed.has(key))
    || !Array.isArray(metadata.recommend_topics) || !metadata.recommend_topics.length || metadata.recommend_topics.length > 24
    || !metadata.recommend_topics.every((topic) => typeof topic === "string" && /^[a-z0-9][a-z0-9-]{0,47}$/.test(topic))
    || new Set(metadata.recommend_topics).size !== metadata.recommend_topics.length
    || !Array.isArray(metadata.data_access) || metadata.data_access.length > 32
    || !metadata.data_access.every((item) => typeof item === "string" && item.trim())) {
    throw new Error(`${name}: invalid feature.toml metadata`);
  }
  if (metadata.data_access.some((item) => (item as string).length > 120)
    || new Set(metadata.data_access).size !== metadata.data_access.length) {
    throw new Error(`${name}: invalid feature.toml data access list`);
  }
  return metadata;
}

function binaryName(name: string): string {
  const manifest = service(name);
  const command = manifest.command as string[];
  if (!Array.isArray(command) || command.length !== 1 || !/^target\/release\/[a-z0-9-]+$/.test(command[0])) {
    throw new Error(`${name}: only a single compiled server command can be packaged`);
  }
  return command[0].split("/").at(-1)!;
}

function binaryManifest(name: string, local: boolean) {
  let text = readFileSync(join(unitDir(name), "service.toml"), "utf8");
  text = text.replace(/^build\s*=.*\n/gm, "");
  if (local) {
    text = text.replace(/^command\s*=.*$/m, `command = ["capabilities/${name}/bin/${binaryName(name)}"]`);
  }
  return text;
}

interface Options {
  version: string;
  out: string;
  target: string;
  kind: "cli" | "node" | "capability" | "macos";
  capability: string;
  build: boolean;
  prepareDependencies: boolean;
  identity: string;
  notary: string;
}

function parse(args: string[]): Options {
  if (!["darwin", "linux"].includes(process.platform)) throw new Error("release preparation supports macOS and Linux hosts");
  const kind = args.shift();
  if (!kind || !["cli", "node", "capability", "macos"].includes(kind)) throw new Error("choose cli, node, capability or macos");
  const values = new Map<string, string>();
  let build = false;
  let prepareDependencies = false;
  while (args.length) {
    const flag = args.shift()!;
    if (flag === "--build") { build = true; continue; }
    if (flag === "--prepare-dependencies") { prepareDependencies = true; continue; }
    if (!["--version", "--out", "--target-dir", "--capability", "--identity", "--notary-profile"].includes(flag) || !args.length) {
      throw new Error(`unknown or incomplete option: ${flag}`);
    }
    if (values.has(flag)) throw new Error(`repeated option: ${flag}`);
    values.set(flag, args.shift()!);
  }
  const version = values.get("--version") ?? "";
  if (!/^v\d+\.\d+\.\d+$/.test(version)) throw new Error("--version vMAJOR.MINOR.PATCH is required");
  const out = values.get("--out");
  if (!out) throw new Error("--out must name a new output directory");
  const output = resolve(out);
  if (existsSync(output)) throw new Error("output directory already exists; choose a new path");
  const capability = values.get("--capability") ?? "calendar";
  if (!/^[a-z0-9][a-z0-9-]*$/.test(capability) || core.includes(capability)) throw new Error("choose an optional capability");
  if (process.platform !== "darwin" && kind === "macos") throw new Error("macos packaging requires macOS");
  const identity = values.get("--identity") ?? "";
  if (identity && !identity.startsWith("Developer ID Application:")) throw new Error("direct distribution requires a Developer ID Application identity");
  const notary = values.get("--notary-profile") ?? "";
  if (notary && !identity) throw new Error("notarization requires --identity");
  if (kind !== "macos" && (identity || notary)) throw new Error("signing options belong to macos packaging");
  const target = resolve(values.get("--target-dir") ?? (build ? join(output, ".build") : join(root, "target")));
  if (build && (target === join(root, "target") || target.startsWith(join(root, "target") + "/")
    || (process.env.CARGO_TARGET_DIR && target === resolve(process.env.CARGO_TARGET_DIR)))) {
    throw new Error("release builds need an isolated --target-dir; do not overwrite deployed binaries or the shared developer cache");
  }
  if (prepareDependencies && kind !== "cli") throw new Error("--prepare-dependencies belongs to CLI packaging");
  return { kind: kind as Options["kind"], version, out: output, target, capability, build, prepareDependencies, identity, notary };
}

function build(options: Options) {
  if (!options.build) return [];
  if (options.kind === "macos") process.env.MACOSX_DEPLOYMENT_TARGET = minimumMacOS();
  const targets = options.kind === "cli" ? cliBinaries : options.kind === "capability" ? [binaryName(options.capability)]
    : options.kind === "macos" ? [...binaries, "sjel-desktop-launcher"] : binaries;
  const command = ["cargo", "build", "--locked", "--release", "--target-dir", options.target, ...targets.flatMap((name) => ["--bin", name])];
  if (options.kind === "cli") {
    const result = Bun.spawnSync([...command, "--message-format=json"], { cwd: root, stdout: "pipe", stderr: "inherit" });
    const output = result.stdout.toString();
    for (const line of output.split("\n").filter(Boolean)) {
      const message = JSON.parse(line);
      if (message.reason === "compiler-message" && message.message?.rendered) console.error(message.message.rendered);
    }
    if (result.exitCode !== 0) throw new Error(`CLI build failed (${result.exitCode})`);
    return compiledRustSources(root, output);
  }
  run(command);
  if (options.kind === "macos") {
    run(["cargo", "build", "--locked", "--release", "--manifest-path", "apps/gpui/Cargo.toml", "--package", "sjel-gpui", "--target-dir", options.target]);
  }
  return [];
}

function runtime(options: Options, stage: string) {
  for (const name of binaries) copy(join(options.target, "release", name), join(stage, "target/release", name));
  for (const name of core) {
    mkdirSync(join(stage, "capabilities", name), { recursive: true });
    const text = service(name).kind === "process" ? binaryManifest(name, false)
      : readFileSync(join(unitDir(name), "service.toml"), "utf8");
    writeFileSync(join(stage, "capabilities", name, "service.toml"), text);
  }
  for (const name of ["sjel.toml", "systems.toml", "upstreams.toml", "profiles.toml", "self.json", "LICENSE", "README.md", "CONTRIBUTING.md", "ARCHITECTURE.md", "sjel"]) {
    if (existsSync(join(root, name))) copy(join(root, name), join(stage, name));
  }
  for (const name of ["capability.sh", "service-runner.sh", "watchdog.sh", "setup-inbound-auth.sh", "codesign-binary.sh", "doctor", "toolchain-check", "model-check"]) {
    copy(join(root, "tools", name), join(stage, "tools", name));
  }
  for (const name of ["lib", "templates"]) copyPublicTree(`tools/${name}`, join(stage, "tools", name));
  for (const name of ["storage/storage", "capability-auth/capability-auth", "fda-launcher/install"]) copy(join(root, "tools", name), join(stage, "tools", name));
  copy(join(root, "toolchain.toml"), join(stage, "toolchain.toml"));
  for (const name of ["schemas", "docs", "website/content"]) copyPublicTree(name, join(stage, name));
  writeFileSync(join(stage, "distribution.json"), JSON.stringify({ schema: 1, version: options.version, platform, core, source_builds: false }, null, 2) + "\n");
  files(stage); // Refuse developer symlinks in the portable payload.
}

function capability(options: Options, stage: string) {
  const payload = join(stage, "payload");
  const binary = binaryName(options.capability);
  copy(join(options.target, "release", binary), join(payload, "bin", binary));
  writeFileSync(join(payload, "service.toml"), binaryManifest(options.capability, true));
  const manifest = {
    schema: 1, name: options.capability, version: options.version, platform,
    requires: service(options.capability).requires ?? [],
    feature: feature(options.capability),
    files: files(payload).map((path) => ({ path, bytes: statSync(join(payload, path)).size,
      sha256: sha(join(payload, path)), executable: (statSync(join(payload, path)).mode & 0o111) !== 0 })),
  };
  writeFileSync(join(stage, "package.json"), JSON.stringify(manifest, null, 2) + "\n");
  const name = `sjel-${options.capability}-${options.version}-${platform}.tar.gz`;
  run(["tar", "-czf", join(options.out, name), "package.json", "payload"], stage);
}

function macos(options: Options, stage: string) {
  const bundle = join(stage, "Sjel.app");
  const contents = join(bundle, "Contents");
  copy(join(options.target, "release/sjel-desktop-launcher"), join(contents, "MacOS/Sjel"));
  copy(join(options.target, "release/sjel-gpui"), join(contents, "Helpers/Sjel Desktop"));
  copy(join(root, "apps/gpui/Resources/AppIcon.icns"), join(contents, "Resources/AppIcon.icns"));
  runtime(options, join(contents, "Resources/runtime"));
  const version = options.version.slice(1);
  writeFileSync(join(contents, "Info.plist"), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.sjel.desktop</string>
<key>CFBundleExecutable</key><string>Sjel</string>
<key>CFBundleName</key><string>Sjel</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>${version}</string>
<key>CFBundleVersion</key><string>${version}</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>LSMinimumSystemVersion</key><string>${minimumMacOS()}</string>
<key>NSHighResolutionCapable</key><true/>
<key>CFBundleURLTypes</key><array><dict><key>CFBundleURLName</key><string>dev.sjel.desktop</string>
<key>CFBundleURLSchemes</key><array><string>sjel</string></array></dict></array>
</dict></plist>\n`);
  const executables = ["MacOS/Sjel", "Helpers/Sjel Desktop", ...binaries.map((name) => `Resources/runtime/target/release/${name}`)];
  for (const name of executables) {
    const path = join(contents, name);
    const dependencies = Bun.spawnSync(["otool", "-L", path], { stdout: "pipe", stderr: "pipe" });
    if (dependencies.exitCode !== 0) throw new Error(`cannot inspect binary: ${name}`);
    const lines = dependencies.stdout.toString().split("\n").slice(1).filter((line) => line.trim());
    if (lines.some((line) => !/^\s+(\/usr\/lib\/|\/System\/Library\/)/.test(line))) {
      throw new Error(`${name}: external dylib must be bundled before distribution`);
    }
    if (options.identity) run(["codesign", "--force", "--options", "runtime", "--timestamp", "--sign", options.identity, path]);
  }
  if (options.identity) {
    run(["codesign", "--force", "--options", "runtime", "--timestamp", "--sign", options.identity, bundle]);
    run(["codesign", "--verify", "--deep", "--strict", bundle]);
  }
  // Every disk image carries its readiness note alongside the app.
  copy(join(options.out, "READINESS.txt"), join(stage, "READINESS.txt"));
  run(["ln", "-s", "/Applications", join(stage, "Applications")]);
  const dmg = join(options.out, `Sjel-${options.version}-${platform}.dmg`);
  run(["hdiutil", "create", "-volname", "Sjel", "-srcfolder", stage, "-format", "UDZO", dmg]);
  if (options.notary) {
    run(["codesign", "--sign", options.identity, "--timestamp", dmg]);
    run(["xcrun", "notarytool", "submit", dmg, "--keychain-profile", options.notary, "--wait"]);
    run(["xcrun", "stapler", "staple", dmg]);
    run(["xcrun", "stapler", "validate", dmg]);
  }
}

function main() {
  if (process.argv.includes("--help")) {
    console.log("bun tools/distribution/prepare.ts <cli|node|capability|macos> --version vX.Y.Z --out NEW_DIR [--build] [--prepare-dependencies] [--target-dir DIR] [--capability calendar] [--identity 'Developer ID Application: …'] [--notary-profile NAME]");
    return;
  }
  const options = parse(process.argv.slice(2));
  const revision = Bun.spawnSync(["git", "rev-parse", "HEAD"], { cwd: root, stdout: "pipe", stderr: "pipe" });
  const changes = Bun.spawnSync(["git", "-c", "core.fsmonitor=false", "status", "--porcelain"], { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (revision.exitCode || changes.exitCode) throw new Error("cannot establish build source identity");
  const source = { repository: "https://github.com/larsboes/Sjel", commit: revision.stdout.toString().trim(),
    dirty: changes.stdout.length !== 0, rebuilt: options.build };
  const cliSelection = options.kind === "cli" ? selectCliSources(root) : undefined;
  const compiledSources = build(options);
  mkdirSync(options.out, { recursive: true });
  if (options.kind === "cli") writeFileSync(join(options.out, "READINESS.txt"), "CLI PREPARATION ARTIFACT — NOT YET CLEARED FOR A FRIEND TEST\nNo CLI installation, explicit self-update or rollback was tested by producing these files.\nLinux clean-user testing is deferred. No node services or desktop app are included.\n" + ((options.build || options.prepareDependencies) ? "Pi production dependencies prepared with frozen lockfiles and lifecycle scripts disabled.\n" : "Fixture only: Pi package dependencies unprepared; dependency-backed packages are omitted and held.\n"));
  else writeFileSync(join(options.out, "READINESS.txt"), "PREPARATION ARTIFACT — NOT YET CLEARED FOR A FRIEND TEST\nNo installation was tested by producing these files.\nMac first-run create/connect-node UI, writable runtime placement, browser login and update/rollback integration must be completed before a clean-user trial.\nUse website/content/operations/index.md#release-preparation-and-later-testing for the owner rehearsal.\n");
  const stage = mkdtempSync(join(tmpdir(), "sjel-release-"));
  try {
    if (options.kind === "cli") {
      const sources = writeCliPayload(root, join(stage, "sjel"), options.target, options.version, platform, options.build || options.prepareDependencies, cliSelection, compiledSources);
      if (options.build) writeCorrespondingSource(root, join(options.out, `sjel-source-${options.version}-${platform}.tar.gz`), stage, sources);
      run(["tar", "-czf", join(options.out, `sjel-cli-${options.version}-${platform}.tar.gz`), "sjel"], stage);
      writeFileSync(join(options.out, "CLI_VERSION"), options.version + "\n");
      const installer = readFileSync(join(root, "tools/distribution/cli-install.sh"), "utf8");
      writeFileSync(join(options.out, "install.sh"), standaloneInstaller(options.version, installer));
      copy(join(root, "tools/distribution/cli-install.sh"), join(options.out, "cli-install.sh"));
    }
    else if (options.kind === "macos") macos(options, stage);
    else if (options.kind === "capability") capability(options, stage);
    else {
      runtime(options, join(stage, "sjel"));
      run(["tar", "-czf", join(options.out, `sjel-node-${options.version}-${platform}.tar.gz`), "sjel"], stage);
    }
    const assets = readdirSync(options.out).filter((name) => /\.(tar\.gz|dmg)$/.test(name) || (options.kind === "cli" && ["CLI_VERSION", "cli-install.sh", "install.sh"].includes(name))).sort();
    const capabilityPackages = options.kind === "capability"
      ? assets.map((name) => ({
        name: options.capability,
        version: options.version,
        platform,
        requires: (service(options.capability).requires as string[] | undefined) ?? [],
        feature: feature(options.capability),
        asset: name,
        bytes: statSync(join(options.out, name)).size,
        sha256: sha(join(options.out, name)),
      }))
      : [];
    const catalogue = { schema: 1, version: options.version, platform, source, friend_ready: false,
      signed: Boolean(options.identity), notarized: Boolean(options.notary),
      assets: assets.map((name) => ({ name, bytes: statSync(join(options.out, name)).size, sha256: sha(join(options.out, name)) })),
      capabilities: capabilityPackages };
    writeFileSync(join(options.out, "release.json"), JSON.stringify(catalogue, null, 2) + "\n");
    writeFileSync(join(options.out, "SHA256SUMS"), assets.map((name) => `${sha(join(options.out, name))}  ${name}\n`).join(""));
    console.log(`Prepared ${assets.length} artifact(s) in ${options.out}; nothing installed or published.`);
  } finally { rmSync(stage, { recursive: true, force: true }); }
}

try { main(); } catch (error) { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; }
