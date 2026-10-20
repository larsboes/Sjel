#!/usr/bin/env bun
// Release orchestration uses the same Bun build runtime as tools/generate-site.ts.
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

const root = resolve(import.meta.dir, "../..");
const core = ["sjel-status", "devices", "entities", "store"];
const binaries = ["sjel-cli", "sjel-status", "devices-server", "entities-server", "sjel-storage", "sjel-capability-auth"];
const platform = `${process.arch === "arm64" ? "aarch64" : process.arch === "x64" ? "x86_64" : process.arch}-${process.platform === "darwin" ? "apple-darwin" : "unknown-linux-gnu"}`;

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

function service(name: string) {
  return Bun.TOML.parse(readFileSync(join(root, "capabilities", name, "service.toml"), "utf8")) as Record<string, unknown>;
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
  let text = readFileSync(join(root, "capabilities", name, "service.toml"), "utf8");
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
  kind: "node" | "capability" | "macos";
  capability: string;
  build: boolean;
  identity: string;
  notary: string;
}

function parse(args: string[]): Options {
  if (!["darwin", "linux"].includes(process.platform)) throw new Error("release preparation supports macOS and Linux hosts");
  const kind = args.shift();
  if (!kind || !["node", "capability", "macos"].includes(kind)) throw new Error("choose node, capability or macos");
  const values = new Map<string, string>();
  let build = false;
  while (args.length) {
    const flag = args.shift()!;
    if (flag === "--build") { build = true; continue; }
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
  return { kind: kind as Options["kind"], version, out: output, target, capability, build, identity, notary };
}

function build(options: Options) {
  if (!options.build) return;
  const targets = options.kind === "capability" ? [binaryName(options.capability)]
    : options.kind === "macos" ? [...binaries, "sjel-desktop-launcher"] : binaries;
  run(["cargo", "build", "--locked", "--release", "--target-dir", options.target, ...targets.flatMap((name) => ["--bin", name])]);
  if (options.kind !== "capability") {
    run(["bun", "install", "--frozen-lockfile", "--ignore-scripts"], join(root, "dashboard"));
    run(["bun", "run", "build"], join(root, "dashboard"));
  }
  if (options.kind === "macos") {
    run(["cargo", "build", "--locked", "--release", "--manifest-path", "apps/gpui/Cargo.toml", "--package", "sjel-gpui", "--target-dir", options.target]);
  }
}

function runtime(options: Options, stage: string) {
  for (const name of binaries) copy(join(options.target, "release", name), join(stage, "target/release", name));
  for (const name of core) {
    mkdirSync(join(stage, "capabilities", name), { recursive: true });
    const text = service(name).kind === "process" ? binaryManifest(name, false)
      : readFileSync(join(root, "capabilities", name, "service.toml"), "utf8");
    writeFileSync(join(stage, "capabilities", name, "service.toml"), text);
  }
  for (const name of ["sjel.toml", "systems.toml", "upstreams.toml", "profiles.toml", "self.json", "LICENSE", "README.md", "CONTRIBUTING.md", "ARCHITECTURE.md", "sjel"]) {
    if (existsSync(join(root, name))) copy(join(root, name), join(stage, name));
  }
  for (const name of ["capability.sh", "service-runner.sh", "watchdog.sh", "setup-inbound-auth.sh", "codesign-binary.sh", "doctor", "toolchain-check", "model-check"]) {
    copy(join(root, "tools", name), join(stage, "tools", name));
  }
  for (const name of ["lib", "templates"]) copyPublicTree(`tools/${name}`, join(stage, "tools", name));
  for (const name of ["storage/storage", "capability-auth/capability-auth"]) copy(join(root, "tools", name), join(stage, "tools", name));
  copy(join(root, "toolchain.toml"), join(stage, "toolchain.toml"));
  for (const name of ["schemas", "docs"]) copyPublicTree(name, join(stage, name));
  copy(join(root, "dashboard/dist"), join(stage, "dashboard/dist"));
  const dashboard = readFileSync(join(root, "dashboard/service.toml"), "utf8")
    .replace(/^build\s*=.*\n/gm, "").replace(/^command\s*=.*$/m, 'command = ["false"]');
  writeFileSync(join(stage, "dashboard/service.toml"), dashboard);
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
<key>LSMinimumSystemVersion</key><string>14.0</string>
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
    console.log("bun tools/distribution/prepare.ts <node|capability|macos> --version vX.Y.Z --out NEW_DIR [--build] [--target-dir DIR] [--capability calendar] [--identity 'Developer ID Application: …'] [--notary-profile NAME]");
    return;
  }
  const options = parse(process.argv.slice(2));
  const revision = Bun.spawnSync(["git", "rev-parse", "HEAD"], { cwd: root, stdout: "pipe", stderr: "pipe" });
  const changes = Bun.spawnSync(["git", "-c", "core.fsmonitor=false", "status", "--porcelain"], { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (revision.exitCode || changes.exitCode) throw new Error("cannot establish build source identity");
  const source = { repository: "https://github.com/larsboes/Sjel", commit: revision.stdout.toString().trim(),
    dirty: changes.stdout.length !== 0, rebuilt: options.build };
  build(options);
  mkdirSync(options.out, { recursive: true });
  writeFileSync(join(options.out, "READINESS.txt"), "PREPARATION ARTIFACT — NOT YET CLEARED FOR A FRIEND TEST\nNo installation was tested by producing these files.\nMac first-run create/connect-node UI, writable runtime placement, browser login and update/rollback integration must be completed before a clean-user trial.\nUse docs/operations/index.md#release-preparation-and-later-testing for the owner rehearsal.\n");
  const stage = mkdtempSync(join(tmpdir(), "sjel-release-"));
  try {
    if (options.kind === "macos") macos(options, stage);
    else if (options.kind === "capability") capability(options, stage);
    else {
      runtime(options, join(stage, "sjel"));
      run(["tar", "-czf", join(options.out, `sjel-node-${options.version}-${platform}.tar.gz`), "sjel"], stage);
    }
    const assets = readdirSync(options.out).filter((name) => /\.(tar\.gz|dmg)$/.test(name));
    const catalogue = { schema: 1, version: options.version, platform, source, friend_ready: false,
      signed: Boolean(options.identity), notarized: Boolean(options.notary),
      assets: assets.map((name) => ({ name, bytes: statSync(join(options.out, name)).size, sha256: sha(join(options.out, name)) })) };
    writeFileSync(join(options.out, "release.json"), JSON.stringify(catalogue, null, 2) + "\n");
    writeFileSync(join(options.out, "SHA256SUMS"), assets.map((name) => `${sha(join(options.out, name))}  ${name}\n`).join(""));
    console.log(`Prepared ${assets.length} artifact(s) in ${options.out}; nothing installed or published.`);
  } finally { rmSync(stage, { recursive: true, force: true }); }
}

try { main(); } catch (error) { console.error(error instanceof Error ? error.message : String(error)); process.exitCode = 1; }
