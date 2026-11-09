import { existsSync, readFileSync, realpathSync, statSync } from "node:fs";
import { extname, join, posix, resolve } from "node:path";

const COMMON = ["tools/lib/**", "sjel.toml", "axon.toml"];
const CARGO = ["**/Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/**", "**/build.rs"];
// sjel-cli's local dependency closure can grow with any workspace manifest change.
const CLI = ["sjel", "crates/sjel-cli/**", "tools/sjel-cli/**", "tools/capability.sh", ...COMMON, ...CARGO, "**/*.rs", "**/*.sql"];
const GENERATOR = ["tools/generate-architecture.sh", "crates/*/README.md", "crates/*/service.toml", "services/*/README.md", "services/*/service.toml", "capabilities/*/README.md", "capabilities/*/service.toml", "Packs/*/pack.toml", "libs/*/README.md"];
const METADATA = ["Packs/*/skills/*/SKILL.md", "Packs/writing/skills/skill-creator/scripts/**"];
const PALETTE = ["Packs/presentations/skills/slide-deck/scripts/deckkit/theme.py", "Packs/presentations/skills/diagrams/scripts/diagramkit/theme.py", "Packs/presentations/skills/slide-deck/assets/themes/**"];
const SELF = ["self.json", "upstreams.toml", "**/service.toml", "crates/*/**", "services/*/**", "capabilities/*/**", "libs/*/**", "Packs/*/**", "schemas/**", "tools/**", ...CLI];

/** Repository gate commands and their read boundaries; publication is never optional or cached. */
export const GATES: Record<string, { cmd: string[]; inputs: string[]; cheap: boolean }> = {
  "generator-inputs-tracked": gate("check-generator-inputs-tracked", [...GENERATOR, "**/.gitignore", ".gitignore"]),
  "architecture-fresh": gate("check-architecture-fresh", [...GENERATOR, "libs/*/**", "ARCHITECTURE.md", "upstreams.toml", "systems.toml"], false),
  "service-tomls": gate("check-service-tomls", ["**/service.toml", "schemas/service.toml.example", "**/*.rs", "**/*.ts", "**/*.py"]),
  "route-schemas": gate("check-route-schemas", ["tools/route-schema-baseline.txt", "crates/*/src/*.rs", "capabilities/*/src/*.rs", "**/service.toml", "Cargo.toml"]),
  "manifest-integrity": gate("check-manifest-integrity", ["**/service.toml", "crates/*/**", "services/*/**", "capabilities/*/**"]),
  "doc-self-references": gate("check-doc-self-references", ["**/*.md"]),
  "doc-links": gate("check-doc-links", ["**/*.md", "tools/check-doc-links.ts"]),
  "skill-metadata": gate("check-skill-metadata", METADATA),
  "skill-metadata-test": gate("check-skill-metadata.test", METADATA, false),
  "presentations-theme-contract": gate("check-presentations-theme-contract", PALETTE),
  "presentations-theme-contract-test": gate("check-presentations-theme-contract.test", PALETTE, false),
  "skill-eval": { cmd: ["tools/skill-eval", "check", "--all"], inputs: ["tools/skill-eval", "tools/skill-eval.ts", "Packs/*/skills/**", "**/fixtures/**", ...COMMON], cheap: false },
  "publication-hygiene": gate("check-publication-hygiene", ["**"]),
  "bun-install-policy": gate("check-bun-install-policy", [".github/workflows/ci.yml", "tools/ci-run.ts", "crates/comms/clipper/README.md", "capabilities/comms/clipper/README.md", "**/package.json", "**/bun.lock", "**/bun.lockb", "**/bunfig.toml"]),
  "store-transactions": gate("check-store-transactions", ["**/*.rs"]),
  "obsidian-plugins": gate("check-obsidian-plugins", ["**/manifest.json", "**/main.js"]),
  self: { cmd: ["tools/self", "check"], inputs: SELF, cheap: false },
  "persistence-orphans": { cmd: ["tools/persistence-orphans.test.sh"], inputs: ["tools/persistence-orphans.test.sh", "tools/doctor", "tools/service-runner.sh", "**/service.toml", "systems.toml", "schemas/**", ...CLI], cheap: false },
};

function gate(name: string, inputs: string[], cheap = true) {
  return { cmd: [`tools/${name}.sh`], inputs: [`tools/${name}.sh`, `tools/${name.replace(/\.test$/, "")}.sh`, ...COMMON, ...inputs], cheap };
}

type Inputs = {
  bun: string[];
  shell: string[];
  ui: string[];
  gates: string[];
  pages: boolean;
  security: { deps: boolean; images: boolean; codeql: string[] };
  reasons: string[];
};

type Node = { edges: Set<string>; reads: Set<string>; opaque: string[] };
type Package = { dir: string; name?: string; manifest: any; check: boolean };
const LANGUAGES = ["actions", "javascript-typescript", "python", "rust"];
const SOURCE = /\.(?:[cm]?[jt]sx?|svelte|vue)$/;
const TEST = /(?:\.test|_test|\.spec|_spec)\.(?:[cm]?[jt]sx?)$/;
const SKIP = new Set([".git", "node_modules"]);
const under = (file: string, dir: string) => dir === "." || file === dir || file.startsWith(`${dir}/`);
const sorted = (values: Iterable<string>) => [...new Set(values)].sort();
const inert = (file: string) => /\.(?:md|mdx|rst|txt)$/i.test(file) &&
  (file.startsWith("docs/") || (!file.includes("/") && /\.(?:md|mdx|rst)$/i.test(file)) || /(?:^|\/)README\./i.test(file)) &&
  !/(?:^|\/)(?:fixtures?|testdata|__snapshots__)(?:\/|$)/i.test(file);
const ignoredTree = (file: string) => file.split("/").some((part) => SKIP.has(part));

function gitFiles(root: string): string[] {
  const run = Bun.spawnSync(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], { cwd: root });
  if (run.exitCode !== 0) throw new Error(`Cannot enumerate CI inputs: ${run.stderr.toString()}`);
  return sorted(run.stdout.toString().split("\0").filter((file) => file && !ignoredTree(file)));
}

/** Locked Bun package directories whose dependencies are required by repository tests.
 * @throws If Git cannot enumerate repository inputs.
 */
export function bunInstallDirs(root: string): string[] {
  const inventory = gitFiles(root);
  const available = new Set(inventory);
  return inventory.filter((file) => posix.basename(file) === "package.json" && existsSync(join(root, file)))
    .map((file) => posix.dirname(file))
    .filter((dir) => ["bun.lock", "bun.lockb"].some((lock) => available.has(posix.join(dir, lock)) && existsSync(join(root, dir, lock))))
    .sort();
}

function read(root: string, file: string): string {
  return readFileSync(join(root, file), "utf8");
}

function config(root: string): { ignore: Bun.Glob[]; preload: string[] } {
  const file = join(root, "bunfig.toml");
  const test = existsSync(file) ? (Bun.TOML.parse(readFileSync(file, "utf8")) as any).test ?? {} : {};
  if (test.root && test.root !== ".") throw new Error("CI input discovery does not support a non-root Bun test.root");
  const ignore = test.pathIgnorePatterns ?? [];
  const preload = typeof test.preload === "string" ? [test.preload] : test.preload ?? [];
  if (!Array.isArray(ignore) || !Array.isArray(preload) || [...ignore, ...preload].some((x) => typeof x !== "string")) {
    throw new Error("Cannot interpret Bun test discovery configuration");
  }
  return { ignore: ignore.map((p: string) => new Bun.Glob(p)), preload };
}

function localPath(file: string, literal: string): string {
  return posix.normalize(posix.join(posix.dirname(file), literal));
}

function resolveImport(file: string, specifier: string, available: Set<string>): string | undefined {
  const path = localPath(file, specifier);
  const candidates = [path, ...[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs", ".json", ".svelte", ".vue"].map((ext) => path + ext),
    ...["ts", "tsx", "js", "jsx", "mts", "mjs"].map((ext) => `${path}/index.${ext}`)];
  if (/\.[cm]?js$/.test(path)) candidates.push(path.replace(/\.[cm]?js$/, ".ts"), path.replace(/\.[cm]?js$/, ".tsx"));
  return candidates.find((p) => available.has(p));
}

function literalReads(source: string, file: string, available: Set<string>): Set<string> {
  const out = new Set<string>();
  for (const match of source.matchAll(/(["'`])([^"'`\n\\]+)\1/g)) {
    const literal = match[2];
    if (literal.includes("${") || !/^[\w.\-/]+\/?$/.test(literal)) continue;
    for (const path of [posix.normalize(literal), localPath(file, literal)]) {
      if (path === "." || path.startsWith("../") || path.startsWith("/")) continue;
      if (available.has(path)) out.add(path);
    }
  }
  return out;
}

function packages(root: string, inventory: string[], available: Set<string>): Package[] {
  const manifests = inventory.filter((file) => posix.basename(file) === "package.json");
  if (!manifests.length) return [];
  const run = Bun.spawnSync(["git", "ls-files", "--cached", "-z"], { cwd: root });
  if (run.exitCode !== 0) throw new Error("Cannot verify UI manifest and lockfile trackedness");
  const tracked = new Set(run.stdout.toString().split("\0"));
  return manifests.map((file) => {
    const manifest = JSON.parse(read(root, file));
    const dir = posix.dirname(file);
    const check = !!manifest.scripts?.check;
    const svelte = Object.hasOwn(manifest.dependencies ?? {}, "svelte") || Object.hasOwn(manifest.devDependencies ?? {}, "svelte") || available.has(posix.join(dir, "svelte.config.js"));
    if (!check && svelte) throw new Error(`${file}: Svelte package has no check script`);
    if (check) {
      const lock = ["bun.lock", "bun.lockb"].map((name) => posix.join(dir, name)).find((path) => available.has(path));
      if (!lock) throw new Error(`${file}: check script requires a Bun lockfile`);
      if (!tracked.has(file) || !tracked.has(lock)) throw new Error(`${file}: UI manifest and lockfile must be tracked`);
    }
    return { dir, manifest, check, name: manifest.name };
  });
}

function graph(root: string, inventory: string[], available: Set<string>, pkgs: Package[]): Map<string, Node> {
  const nodes = new Map<string, Node>();
  const named = new Set(available);
  for (const file of inventory) {
    let parent = posix.dirname(file);
    while (parent !== ".") { named.add(parent); parent = posix.dirname(parent); }
  }
  for (const file of inventory.filter((p) => SOURCE.test(p) || /\.(?:jsonc?|sh|bash)$/.test(p) || (!extname(p) && under(p, "tools")))) {
    const source = read(root, file);
    const node: Node = { edges: new Set(), reads: literalReads(source, file, named), opaque: [] };
    nodes.set(file, node);
    if (/\.jsonc?$/.test(file)) {
      for (const match of source.matchAll(/["'](\.[^"'\n]+)["']/g)) {
        const target = resolveImport(file, match[1], available);
        if (target) node.edges.add(target);
      }
      continue;
    }
    if (!SOURCE.test(file)) {
      for (const match of source.matchAll(/(?:^|[\s"'])(?:\$\{?SJEL_ROOT\}?\/)?(tools\/[\w./-]+)/gm)) {
        node.reads.add(match[1]);
        if (available.has(match[1])) node.edges.add(match[1]);
      }
      // Shell expansion, sourced variables and command substitution cannot be resolved statically.
      if (/(?:\$|`|\b(?:source|eval|exec|find|cat|read|bash|sh|python3?|bun)\b|(?:^|\s)\.\s)/m.test(source)) node.opaque.push("shell runtime references");
      continue;
    }
    const scanSource = /\.(?:svelte|vue)$/.test(file)
      ? [...source.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)].map((m) => m[1]).join("\n") : source;
    try {
      const loader = /\.[cm]?jsx?$/.test(file) ? "jsx" : "tsx";
      const imports = new Bun.Transpiler({ loader }).scanImports(scanSource);
      for (const imp of imports) {
        const specifier = imp.path;
        if (specifier.startsWith(".")) {
          const target = resolveImport(file, specifier, available);
          if (target) node.edges.add(target);
          else node.opaque.push(`unresolved import ${specifier}`);
        } else if (!specifier.startsWith("node:") && specifier !== "bun:test" && specifier !== "bun") {
          const dep = pkgs.find((p) => p.name && (specifier === p.name || specifier.startsWith(`${p.name}/`)));
          if (dep) {
            node.reads.add(dep.dir);
            for (const path of inventory) if (under(path, dep.dir)) node.edges.add(path);
          } else node.opaque.push(`external package runtime ${specifier}`);
        }
      }
    } catch {
      node.opaque.push("unparseable imports");
    }
    for (const call of scanSource.matchAll(/\b(?:import|require)\s*\(([^)]*)\)/g)) {
      if (!/^\s*(["'])[^"'\n\\]*\1\s*$/.test(call[1])) node.opaque.push("computed import");
    }
    if (/\b(?:createRequire|require\.resolve|module\.require)\b|\brequire\b(?!\s*\()/.test(scanSource)) node.opaque.push("indirect module loading");
    if (/\b(?:eval|Function)\s*\(|\b(?:spawn(?:Sync)?|exec(?:File)?(?:Sync)?)\s*\(|\bBun\.\$|\$\s*`/.test(scanSource) || /(?:child_process|bun:ffi)/.test(scanSource)) {
      node.opaque.push("subprocess or runtime evaluation");
    }
    // Even a literal read can be affected by cwd, symlinks, aliases or a composed path.
    if (/(?:node:fs|["']fs(?:\/promises)?["']|["']bun["']|\bBun\b|\b(?:readFile(?:Sync)?|readdir(?:Sync)?|createReadStream|readlink(?:Sync)?|glob(?:Sync)?|open(?:Sync)?)\s*\()/.test(scanSource)) {
      node.opaque.push("runtime file reads");
    }
    if (/\b(?:from|import)\b/.test(source) && /(?:tsconfig|vite\.config|svelte\.config)/.test(file)) node.opaque.push("build configuration imports");
  }
  for (const node of nodes.values()) {
    for (const path of node.reads) {
      if (nodes.has(path)) node.edges.add(path);
    }
  }
  return nodes;
}

function closure(seeds: Iterable<string>, nodes: Map<string, Node>): Set<string> {
  const seen = new Set<string>();
  const visit = (file: string) => {
    if (seen.has(file)) return;
    seen.add(file);
    for (const edge of nodes.get(file)?.edges ?? []) visit(edge);
  };
  for (const seed of seeds) visit(seed);
  return seen;
}

function touches(paths: Set<string>, changes: string[], nodes: Map<string, Node>): boolean {
  return changes.some((file) => paths.has(file) || [...paths].some((p) => [...(nodes.get(p)?.reads ?? [])].some((r) => under(file, r))));
}

function opaque(paths: Set<string>, nodes: Map<string, Node>): string | undefined {
  for (const file of paths) {
    const why = nodes.get(file)?.opaque[0];
    if (why) return `${file}: ${why}`;
  }
}

function uiPaths(pkg: Package, pkgs: Package[], inventory: string[], nodes: Map<string, Node>): Set<string> {
  const deps = new Set([pkg.dir]);
  let size = -1;
  while (size !== deps.size) {
    size = deps.size;
    for (const owner of pkgs.filter((p) => deps.has(p.dir))) {
      const dependencies = { ...owner.manifest.dependencies, ...owner.manifest.devDependencies, ...owner.manifest.optionalDependencies, ...owner.manifest.peerDependencies };
      for (const [name, version] of Object.entries(dependencies)) {
        const local = pkgs.find((p) => p.name === name) ?? (typeof version === "string" && /^(?:file:|link:)/.test(version)
          ? pkgs.find((p) => p.dir === posix.normalize(posix.join(owner.dir, version.replace(/^(?:file:|link:)/, "")))) : undefined);
        if (local) deps.add(local.dir);
        else if (typeof version === "string" && /^(?:workspace:|file:|link:)/.test(version)) throw new Error(`${owner.dir}: unresolved local dependency ${name}`);
      }
    }
  }
  return closure(inventory.filter((file) => [...deps].some((dir) => under(file, dir))), nodes);
}

function pagesInput(file: string): boolean {
  // Pages checks self before rendering; self scans manifests, Cargo coupling and source routes.
  return /^(?:website\/|brand\/|\.github\/assets\/|\.github\/workflows\/pages\.ya?ml$)/.test(file) ||
    /(?:^|\/)(?:README\.md|Cargo\.(?:toml|lock)|service\.toml|pack\.toml)$/.test(file) ||
    /^(?:self\.json|upstreams\.toml|systems\.toml|sjel\.toml|rust-toolchain\.toml|ARCHITECTURE\.md)$/.test(file) ||
    /^(?:crates\/|services\/|capabilities\/|Packs\/|libs\/|tools\/|\.cargo\/)/.test(file) || /\.rs$/.test(file);
}

/** Registered full gate set, independent of test and package discovery.
 * @throws If a newly added gate has no command and input declaration.
 */
export function allGateIDs(root: string): string[] {
  const commands = new Set(Object.values(GATES).map((gate) => gate.cmd[0]));
  for (const file of gitFiles(root)) {
    if (/^tools\/check-.*\.sh$/.test(file) && !file.endsWith(".test.sh") && file !== "tools/check-site-payload.sh" && !commands.has(file)) {
      throw new Error(`Unregistered repository gate: ${file}; add its command and inputs to GATES`);
    }
  }
  return Object.keys(GATES).sort();
}

/**
 * Select repo-relative test files, UI directories, gate IDs and peripheral jobs from Git inputs.
 * No selected target is cached here. Unresolvable execution widens selection; malformed discovery
 * or an unregistered gate throws so callers must fail closed. `full` includes scheduled checks.
 */
export function verificationInputs(root: string, files: string[], deleted: string[], full: boolean): Inputs {
  root = realpathSync(resolve(root));
  const inventory = gitFiles(root).filter((file) => existsSync(join(root, file)) && statSync(join(root, file)).isFile());
  const available = new Set(inventory);
  const changed = sorted([...files, ...deleted].map((p) => posix.normalize(p.replace(/^\.\//, ""))));
  if (changed.some((p) => p.startsWith("../") || p.startsWith("/"))) throw new Error("CI inputs must be repository-relative paths");
  allGateIDs(root);
  const discovery = config(root);
  const tests = inventory.filter((file) => TEST.test(file) && !discovery.ignore.some((glob) => glob.match(file)));
  const shell = inventory.filter((file) => /^tools\/[^/]+\.test\.sh$/.test(file));
  const pkgs = packages(root, inventory, available);
  const reasons: string[] = [];
  const planner = changed.find((file) => /^(?:tools\/(?:ci-[^/]+|lib\/ci-[^/]+)|\.github\/workflows\/ci\.ya?ml)$/.test(file));
  const all = full || !!planner;
  if (all) reasons.push(full ? "full verification requested" : `${planner}: verification planner or workflow changed`);
  const nodes = graph(root, inventory, available, pkgs);
  const preload = discovery.preload.map((p) => posix.normalize(p));
  for (const file of preload) if (!available.has(file)) throw new Error(`Unresolved Bun preload: ${file}`);
  const nonProse = changed.filter((file) => !inert(file));
  const globalBun = changed.some((file) => /(?:^|\/)(?:bunfig\.toml|bun\.lockb?|package(?:-lock)?\.json|[jt]sconfig[^/]*\.json)$/.test(file));
  const testPaths = new Map(tests.map((test) => [test, closure([test, ...preload], nodes)]));
  const shellPaths = new Map(shell.map((test) => [test, closure([test], nodes)]));
  const packagePaths = new Map(pkgs.filter((p) => p.check).map((pkg) => [pkg.dir, uiPaths(pkg, pkgs, inventory, nodes)]));
  const known = new Set<string>();
  for (const paths of [...testPaths.values(), ...shellPaths.values(), ...packagePaths.values()]) {
    for (const file of changed) if (touches(paths, [file], nodes)) known.add(file);
  }
  const unknown = nonProse.find((file) => !known.has(file));
  const removed = deleted.find((file) => !inert(file));
  const sharedShell = changed.find((file) => file.startsWith("tools/lib/"));
  const uncertainty = [...testPaths.values()].map((paths) => opaque(paths, nodes)).find(Boolean);
  const shellUncertainty = [...shellPaths.values()].map((paths) => opaque(paths, nodes)).find(Boolean);
  // Unreferenced prose is the only input class exempt from opaque runtime widening.
  const widenBun = all || globalBun || !!removed || !!unknown || (!!uncertainty && nonProse.length > 0);
  const widenShell = all || !!removed || !!unknown || !!sharedShell || (!!shellUncertainty && nonProse.length > 0);
  if (unknown) reasons.push(`${unknown}: unowned executable or data input; widen Bun and shell suites`);
  if (removed) reasons.push(`${removed}: deleted or renamed input; widen affected suites`);
  if (globalBun) reasons.push("Bun configuration, dependency or compiler input changed");
  if (sharedShell) reasons.push(`${sharedShell}: shared shell support changed`);
  if (uncertainty && nonProse.length) reasons.push(`${uncertainty}; widen Bun suite`);
  if (shellUncertainty && nonProse.length) reasons.push(`${shellUncertainty}; widen shell suite`);
  const selectedBun = widenBun ? tests : tests.filter((test) => touches(testPaths.get(test)!, changed, nodes));
  const selectedShell = widenShell ? shell : shell.filter((test) => touches(shellPaths.get(test)!, changed, nodes));
  const selectedUi = new Set<string>();
  const globalUi = changed.some((file) => /^(?:bunfig\.toml|bun\.lockb?|package\.json|[jt]sconfig[^/]*\.json|tools\/discover-ui-packages)$/.test(file));
  for (const pkg of pkgs.filter((p) => p.check)) {
    const paths = packagePaths.get(pkg.dir)!;
    const ambiguous = opaque(paths, nodes);
    if (all || globalUi || removed || unknown || touches(paths, changed, nodes) || (ambiguous && nonProse.length)) {
      selectedUi.add(pkg.dir);
      if (ambiguous && nonProse.length) reasons.push(`${pkg.dir}: ${ambiguous}; widen UI package check`);
    }
  }
  const codeql = new Set<string>();
  let deps = all;
  let images = all;
  let pages = all;
  for (const file of changed) {
    if (pagesInput(file)) pages = true;
    if (/(?:^|\/)(?:Cargo\.(?:toml|lock)|package(?:-lock)?\.json|bun\.lockb?|yarn\.lock|pnpm-lock\.yaml|pyproject\.toml|requirements[^/]*\.txt|uv\.lock|poetry\.lock|Pipfile(?:\.lock)?|go\.(?:mod|sum)|Gemfile(?:\.lock)?|composer\.(?:json|lock)|bunfig\.toml|osv-scanner[^/]*\.toml)$/.test(file)) deps = true;
    if (/(?:^|\/)service\.toml$|(?:service.*schema|schema.*service)|(?:^|\/)\.?grype[^/]*\.(?:ya?ml|toml)$|(?:^|\/)Dockerfile[^/]*$|^tools\/(?:check-service-tomls\.sh|lib\/(?:toml|schedule)\.sh)$/.test(file)) images = true;
    if (/\.rs$|(?:^|\/)Cargo\.(?:toml|lock)$|(?:^|\/)(?:rust-toolchain|clippy)\.toml$|^\.cargo\//.test(file)) codeql.add("rust");
    if (/\.[cm]?[jt]sx?$|(?:^|\/)(?:package(?:-lock)?\.json|bun\.lockb?|bunfig\.toml|[jt]sconfig[^/]*\.json|yarn\.lock|pnpm-lock\.yaml)$/.test(file)) codeql.add("javascript-typescript");
    if (/\.(?:py|pyi|pyw)$|(?:^|\/)(?:pyproject\.toml|requirements[^/]*\.txt|uv\.lock|poetry\.lock|Pipfile(?:\.lock)?|setup\.(?:py|cfg)|tox\.ini)$/.test(file)) codeql.add("python");
    if (/^\.github\/(?:workflows|actions)\/|(?:^|\/)action\.ya?ml$/.test(file)) codeql.add("actions");
    if (/^\.github\/workflows\/security\.ya?ml$/.test(file)) { deps = true; images = true; }
    if (/^\.github\/.*codeql|\.(?:ql|qll)$|(?:^|\/)qlpack\.yml$/.test(file)) for (const language of LANGUAGES) codeql.add(language);
  }
  if (all) for (const language of LANGUAGES) codeql.add(language);
  const declaredExecutable = (file: string) => Object.values(GATES).some((gate) => gate.cmd[0] === file || gate.inputs.includes(file)) ||
    under(file, "tools/lib") || under(file, "crates/sjel-cli") || under(file, "tools/sjel-cli") ||
    under(file, "Packs/writing/skills/skill-creator/scripts");
  const unownedExecutable = nonProse.find((file) => !known.has(file) && !declaredExecutable(file) &&
    (SOURCE.test(file) || /\.(?:rs|sh|bash|py|swift)$/.test(file) || !extname(file) ||
      (available.has(file) && (statSync(join(root, file)).mode & 0o111) !== 0)));
  if (unownedExecutable) reasons.push(`${unownedExecutable}: unknown executable input; widen repository gates`);
  const evalFixtures = new Set<string>();
  for (const suite of inventory.filter((file) => /^Packs\/[^/]+\/skills\/[^/]+\/evals\/evals\.json$/.test(file))) {
    try {
      const parsed = JSON.parse(read(root, suite));
      for (const entry of parsed.evals ?? []) for (const file of entry.files ?? []) {
        if (typeof file === "string") evalFixtures.add(posix.normalize(posix.join(posix.dirname(posix.dirname(suite)), file)));
      }
    } catch {
      if (changed.length) evalFixtures.add(".");
      reasons.push(`${suite}: cannot analyze eval fixtures; select suite validation`);
    }
  }
  const gates = Object.entries(GATES).filter(([id, gate]) => all || !!unownedExecutable || id === "publication-hygiene" ||
    (id === "skill-eval" && changed.some((file) => [...evalFixtures].some((fixture) => under(file, fixture)))) ||
    changed.some((file) => gate.inputs.some((pattern) => new Bun.Glob(pattern).match(file)))).map(([id]) => id).sort();
  if (!changed.length && !all) reasons.push("no changed paths; publication hygiene remains mandatory");
  else if (!selectedBun.length && !selectedShell.length && !selectedUi.size) reasons.push("no test or UI dependency targets selected");
  return { bun: sorted(selectedBun), shell: sorted(selectedShell), ui: sorted(selectedUi), gates, pages, security: { deps, images, codeql: sorted(codeql) }, reasons: sorted(reasons) };
}
