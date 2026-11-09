import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { completeJavaScriptNotices, dependencyNames, registrySources, writeCorrespondingSource, writeRustNotices } from "./cli-notices.ts";

const temporary: string[] = [];
afterEach(() => { for (const path of temporary.splice(0)) rmSync(path, { recursive: true, force: true }); });
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "sjel-notices-test-"));
  temporary.push(root);
  const directory = join(root, "registry", "src", "index.crates.io-test", "sample-1.2.3");
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, "Cargo.toml"), '[package]\nname = "sample"\nversion = "1.2.3"\nlicense = "MIT"\n');
  writeFileSync(join(directory, "LICENSE"), "Complete upstream permission and copyright notice\n");
  writeFileSync(join(directory, "README.md"), "Upstream attribution\n");
  writeFileSync(join(root, "Cargo.lock"), '[[package]]\nname = "sample"\nversion = "1.2.3"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n');
  return { root, directory, sources: () => registrySources(root, "sample v1.2.3|MIT\n", root) };
}

test("dependency rows deduplicate repeated packages and reject incomplete inventories", () => {
  expect(dependencyNames("sample v1.2.3|MIT\nsample v1.2.3 (*)|MIT\nlocal v0.1.0 (/checkout/crates/local)|AGPL-3.0-only\n")).toHaveLength(2);
  expect(() => dependencyNames("")).toThrow("empty");
  expect(() => dependencyNames("unrecognized output")).toThrow("unrecognized");
});

test("locked public registry sources retain full notices and attribution", () => {
  const { root, directory, sources } = fixture();
  mkdirSync(join(directory, "LICENSES"));
  writeFileSync(join(directory, "LICENSES", "Apache-2.0.txt"), "Additional upstream grant\n");
  const output = join(root, "payload");
  mkdirSync(output);
  writeRustNotices(output, sources(), "v0.0.3", "example-platform");
  expect(readFileSync(join(output, "THIRD_PARTY", "rust", "sample-1.2.3", "LICENSE"), "utf8")).toBe("Complete upstream permission and copyright notice\n");
  expect(readFileSync(join(output, "THIRD_PARTY.md"), "utf8")).toContain("sjel-source-v0.0.3-example-platform.tar.gz");
  expect(readFileSync(join(output, "THIRD_PARTY", "rust", "sample-1.2.3", "README.md"), "utf8")).toContain("attribution");
  expect(readFileSync(join(output, "THIRD_PARTY", "rust", "sample-1.2.3", "LICENSES", "Apache-2.0.txt"), "utf8")).toContain("Additional upstream grant");
});

test("unknown registry sources and absent cached crates fail closed", () => {
  const { root, directory, sources } = fixture();
  rmSync(directory, { recursive: true });
  expect(sources).toThrow("source missing");
  writeFileSync(join(root, "Cargo.lock"), '[[package]]\nname = "sample"\nversion = "1.2.3"\nsource = "git+https://example.org/dependency"\n');
  expect(sources).toThrow("unsupported dependency source");
});

test("missing license texts and source symlinks cannot enter notices", () => {
  const { root, directory, sources } = fixture();
  rmSync(join(directory, "LICENSE"));
  expect(() => writeRustNotices(root, sources(), "v0.0.3", "example")).toThrow("no license notice");
  symlinkSync(join(directory, "README.md"), join(directory, "LICENSE"));
  expect(() => writeRustNotices(root, sources(), "v0.0.3", "example")).toThrow("symlink");
});

test("license-file paths cannot escape their package", () => {
  const { root, directory, sources } = fixture();
  writeFileSync(join(directory, "Cargo.toml"), '[package]\nlicense = "MIT"\nlicense-file = "../outside"\n');
  expect(() => writeRustNotices(root, sources(), "v0.0.3", "example")).toThrow("escapes");
});

test("JavaScript notices preserve upstream files and supplement only the reviewed missing grant", () => {
  const { root } = fixture();
  const packageRoot = join(root, "javascript");
  const dependency = join(packageRoot, "node_modules", "boolbase");
  mkdirSync(dependency, { recursive: true });
  writeFileSync(join(dependency, "package.json"), JSON.stringify({ name: "boolbase", version: "1.0.0", license: "ISC" }));
  completeJavaScriptNotices([packageRoot]);
  expect(readFileSync(join(dependency, "LICENSE"), "utf8")).toContain("2014-2015, Felix Boehm");
  writeFileSync(join(dependency, "LICENSE"), "Upstream notice retained unchanged\n");
  completeJavaScriptNotices([packageRoot]);
  expect(readFileSync(join(dependency, "LICENSE"), "utf8")).toBe("Upstream notice retained unchanged\n");
});

test("new JavaScript licenses or missing grants require review", () => {
  const { root } = fixture();
  const packageRoot = join(root, "javascript");
  const dependency = join(packageRoot, "node_modules", "sample");
  mkdirSync(dependency, { recursive: true });
  writeFileSync(join(dependency, "package.json"), JSON.stringify({ name: "sample", version: "1.0.0", license: "MIT" }));
  expect(() => completeJavaScriptNotices([packageRoot])).toThrow("license notice missing");
  writeFileSync(join(dependency, "package.json"), JSON.stringify({ name: "sample", version: "1.0.0", license: "UNREVIEWED" }));
  expect(() => completeJavaScriptNotices([packageRoot])).toThrow("unreviewed");
});

test("corresponding source includes the committed tree and complete registry source, never ignored files", () => {
  const { root, directory, sources } = fixture();
  for (const args of [["init", "-q"], ["config", "user.name", "Fixture"], ["config", "user.email", "fixture@example.org"]]) {
    expect(Bun.spawnSync(["git", ...args], { cwd: root }).exitCode).toBe(0);
  }
  writeFileSync(join(root, ".gitignore"), "private-data\nregistry/\n");
  writeFileSync(join(root, "private-data"), "must not be distributed");
  writeFileSync(join(directory, "lib.rs"), "pub fn upstream_source() {}\n");
  expect(Bun.spawnSync(["git", "add", ".gitignore", "Cargo.lock"], { cwd: root }).exitCode).toBe(0);
  expect(Bun.spawnSync(["git", "commit", "-qm", "fixture"], { cwd: root }).exitCode).toBe(0);
  const output = join(root, "source.tar.gz");
  writeCorrespondingSource(root, output, root, sources());
  const listing = Bun.spawnSync(["tar", "-tzf", output], { stdout: "pipe" }).stdout.toString();
  expect(listing).toContain("sjel-source/Cargo.lock");
  expect(listing).toContain("sjel-source/vendor/sample-1.2.3/lib.rs");
  expect(listing).not.toContain("private-data");
});
