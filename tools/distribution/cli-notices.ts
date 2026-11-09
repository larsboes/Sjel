import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";

export interface RustSource { name: string; version: string; license: string; directory: string }

export function dependencyNames(tree: string): Array<{ name: string; version: string }> {
  const packages = new Map<string, { name: string; version: string }>();
  for (const line of tree.split("\n").filter(Boolean)) {
    const match = /^([A-Za-z0-9_-]+) v([0-9][^ ()|]*)(?: \(\*\))?(?: \([^)]*\))?\|/.exec(line);
    if (!match) throw new Error(`unrecognized Cargo dependency row: ${line}`);
    packages.set(`${match[1]}-${match[2]}`, { name: match[1], version: match[2] });
  }
  if (!packages.size) throw new Error("Cargo dependency inventory is empty");
  return [...packages.values()].sort((a, b) => `${a.name}-${a.version}`.localeCompare(`${b.name}-${b.version}`));
}

export function registrySources(root: string, tree: string, cargoHome: string): RustSource[] {
  const lock = Bun.TOML.parse(readFileSync(join(root, "Cargo.lock"), "utf8"));
  const registry = join(cargoHome, "registry", "src");
  const indexes = readdirSync(registry).filter((name) => name.startsWith("index.crates.io-"));
  return dependencyNames(tree).flatMap(({ name, version }) => {
    const packages = lock.package.filter((entry: any) => entry.name === name && entry.version === version);
    if (packages.length !== 1) throw new Error(`ambiguous or missing locked dependency: ${name} ${version}`);
    if (!packages[0].source) return [];
    if (packages[0].source !== "registry+https://github.com/rust-lang/crates.io-index") throw new Error(`unsupported dependency source: ${name}`);
    const directory = indexes.map((index) => join(registry, index, `${name}-${version}`)).find((path) => existsSync(join(path, "Cargo.toml")));
    if (!directory) throw new Error(`crate source missing: ${name} ${version}`);
    const manifest = Bun.TOML.parse(readFileSync(join(directory, "Cargo.toml"), "utf8")).package;
    const license = manifest.license;
    if (typeof license !== "string" || !license.trim()) throw new Error(`crate license missing: ${name}`);
    return [{ name, version, license, directory }];
  });
}

function noticeFiles(directory: string, prefix = ""): string[] {
  return readdirSync(join(directory, prefix), { withFileTypes: true }).flatMap((entry) => {
    const path = join(prefix, entry.name);
    if (entry.isSymbolicLink()) throw new Error(`crate source contains a symlink: ${path}`);
    if (entry.isDirectory()) return noticeFiles(directory, path);
    const notice = /(^|[/\\])(licen[cs]es?|copying|notices?|copyrights?)([._/\\-]|$)/i.test(path);
    return notice || /^readme([._-]|$)/i.test(entry.name) ? [path] : [];
  }).sort();
}

export function writeRustNotices(stage: string, sources: RustSource[], version: string, platform: string) {
  if (!sources.length) throw new Error("Rust dependency notices cannot be empty");
  const rows = ["# Rust dependency notices", "", `Corresponding source: sjel-source-${version}-${platform}.tar.gz in the same GitHub Release.`, "", "JavaScript dependency notices remain alongside each bundled package in node_modules.", ""];
  for (const source of sources) {
    const manifest = Bun.TOML.parse(readFileSync(join(source.directory, "Cargo.toml"), "utf8")).package;
    const paths = noticeFiles(source.directory);
    if (manifest["license-file"]) {
      const path = resolve(source.directory, manifest["license-file"]);
      if (!path.startsWith(resolve(source.directory) + sep)) throw new Error(`license file escapes crate: ${source.name}`);
      const relative = path.slice(resolve(source.directory).length + 1);
      if (!paths.includes(relative)) paths.push(relative);
    }
    if (!paths.some((path) => /license|licence|copying|copyright|notice/i.test(path))) throw new Error(`crate has no license notice: ${source.name}`);
    const id = `${source.name}-${source.version}`;
    rows.push(`## ${id}`, "", `SPDX: ${source.license}`, `Source: https://crates.io/api/v1/crates/${source.name}/${source.version}/download`, "");
    for (const path of paths) {
      const destination = join(stage, "THIRD_PARTY", "rust", id, path);
      mkdirSync(dirname(destination), { recursive: true });
      cpSync(join(source.directory, path), destination);
      rows.push(`- THIRD_PARTY/rust/${id}/${path}`);
    }
    rows.push("");
  }
  writeFileSync(join(stage, "THIRD_PARTY.md"), rows.join("\n") + "\n");
}

export function completeJavaScriptNotices(directories: string[]) {
  const reviewed = new Set(["MIT", "ISC", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "0BSD", "BlueOak-1.0.0", "Unlicense"]);
  function visit(modules: string) {
    for (const entry of readdirSync(modules, { withFileTypes: true })) {
      if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
      const directory = join(modules, entry.name);
      if (entry.name.startsWith("@")) { visit(directory); continue; }
      if (!existsSync(join(directory, "package.json"))) continue;
      const manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8"));
      if (!reviewed.has(manifest.license)) throw new Error(`unreviewed JavaScript license: ${manifest.name}`);
      if (!readdirSync(directory).some((name) => /^(license|licence|copying|notice|copyright)([._-]|$)/i.test(name))) {
        if (manifest.name !== "boolbase" || manifest.version !== "1.0.0" || manifest.license !== "ISC") throw new Error(`JavaScript license notice missing: ${manifest.name}`);
        cpSync(join(import.meta.dir, "notices/boolbase-1.0.0.LICENSE"), join(directory, "LICENSE"));
      }
      if (existsSync(join(directory, "node_modules"))) visit(join(directory, "node_modules"));
    }
  }
  for (const directory of directories) visit(join(directory, "node_modules"));
}

export function collectRustSources(root: string, platform: string): RustSource[] {
  const result = Bun.spawnSync(["cargo", "tree", "--locked", "--offline", "--edges", "normal,build", "--target", platform,
    "--prefix", "none", "--format", "{p}|{l}", "-p", "sjel-cli", "-p", "sjel-claude-config", "-p", "sjel-mcp"], { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (result.exitCode) throw new Error(`Cargo dependency inventory failed: ${result.stderr.toString()}`);
  return registrySources(root, result.stdout.toString(), process.env.CARGO_HOME ?? join(homedir(), ".cargo"));
}

export function writeCorrespondingSource(root: string, output: string, scratch: string, sources: RustSource[]) {
  const archive = join(scratch, "tracked-source.tar");
  const source = join(scratch, "sjel-source");
  mkdirSync(source);
  for (const command of [["git", "archive", "--format=tar", "--output", archive, "HEAD"], ["tar", "-xf", archive, "-C", source]]) {
    const result = Bun.spawnSync(command, { cwd: root, stdout: "inherit", stderr: "inherit" });
    if (result.exitCode) throw new Error("cannot archive tracked corresponding source");
  }
  for (const crate of sources) cpSync(crate.directory, join(source, "vendor", `${crate.name}-${crate.version}`), { recursive: true, dereference: false });
  writeFileSync(join(source, "SOURCE.md"), "# Corresponding source\n\nSjel source is the release commit. Rust dependency source for this platform is under vendor/.\nCargo.lock pins all dependencies. The binary archive retains the executable JavaScript dependencies and their notices.\nBuild the CLI binaries with cargo build --locked --release --bin sjel-cli --bin sjel-claude-config --bin sjel-mcp.\n");
  const result = Bun.spawnSync(["tar", "-czf", output, "sjel-source"], { cwd: scratch, stdout: "inherit", stderr: "inherit" });
  if (result.exitCode) throw new Error("cannot compress corresponding source");
}
