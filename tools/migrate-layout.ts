#!/usr/bin/env bun
/**
 * migrate-layout.ts — Restructure repository layout per docs/plans/restructure.md §3.
 *
 * Moves Rust crates from capabilities/, libs/, and tools/ into a flat crates/* directory,
 * and moves non-Rust supervised services into services/*.
 * Rewrites relative paths in Cargo.toml files, manifest lookup in paths.sh/paths.rs, and
 * architecture generators.
 *
 * Usage:
 *   bun tools/migrate-layout.ts [--dry-run]
 */

import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { execSync } from "node:child_process";

const root = join(import.meta.dir, "..");
const dryRun = process.argv.includes("--dry-run");

function run(cmd: string) {
  console.log(`$ ${cmd}`);
  if (!dryRun) {
    execSync(cmd, { cwd: root, stdio: "inherit" });
  }
}

function replaceInFile(filePath: string, target: string | RegExp, replacement: string) {
  if (!existsSync(filePath)) return;
  const original = readFileSync(filePath, "utf8");
  const updated = typeof target === "string"
    ? original.replaceAll(target, replacement)
    : original.replace(target, replacement);
  if (original !== updated) {
    console.log(`Updated ${filePath}`);
    if (!dryRun) {
      writeFileSync(filePath, updated, "utf8");
    }
  }
}

// 1. Identify all crates and services
const capabilitiesDir = join(root, "capabilities");
const libsDir = join(root, "libs");
const toolsDir = join(root, "tools");

interface MovePlan {
  from: string;
  to: string;
  kind: "crate" | "service";
}

const moves: MovePlan[] = [];

// Capabilities
if (existsSync(capabilitiesDir)) {
  for (const entry of readdirSync(capabilitiesDir)) {
    const full = join(capabilitiesDir, entry);
    if (!existsSync(join(full, "service.toml")) && !existsSync(join(full, "Cargo.toml"))) {
      continue;
    }
    const isCrate = existsSync(join(full, "Cargo.toml"));
    if (isCrate) {
      moves.push({
        from: `capabilities/${entry}`,
        to: `crates/${entry}`,
        kind: "crate",
      });
    } else {
      moves.push({
        from: `capabilities/${entry}`,
        to: `services/${entry}`,
        kind: "service",
      });
    }
  }
}

// Libs
if (existsSync(libsDir)) {
  for (const entry of readdirSync(libsDir)) {
    const full = join(libsDir, entry);
    if (existsSync(join(full, "Cargo.toml"))) {
      // If name is 'agent', destination is 'crates/sjel-agent' to avoid collision with 'crates/agent'
      const dest = entry === "agent" ? "sjel-agent" : entry;
      moves.push({
        from: `libs/${entry}`,
        to: `crates/${dest}`,
        kind: "crate",
      });
    }
  }
}

// Rust tools
const rustTools = [
  "sjel-cli",
  "sjel-mcp",
  "storage",
  "capability-auth",
  "claude-code-config",
  "fda-launcher",
];

for (const tool of rustTools) {
  if (existsSync(join(toolsDir, tool, "Cargo.toml"))) {
    moves.push({
      from: `tools/${tool}`,
      to: `crates/${tool}`,
      kind: "crate",
    });
  }
}

console.log(`Identified ${moves.length} moves:`);
for (const m of moves) {
  console.log(`  ${m.from} -> ${m.to} (${m.kind})`);
}

// 2. Perform git mv
console.log("\n--- Creating target directories and moving ---");
if (!dryRun) {
  mkdirSync(join(root, "crates"), { recursive: true });
  mkdirSync(join(root, "services"), { recursive: true });
}

for (const m of moves) {
  run(`git mv ${m.from} ${m.to}`);
}

// 2b. Move launcher scripts back to tools/
console.log("\n--- Restoring tool launchers to tools/ ---");
const toolLaunchers = [
  { tool: "sjel-mcp", launcher: "sjel-mcp" },
  { tool: "storage", launcher: "storage" },
  { tool: "capability-auth", launcher: "capability-auth" },
  { tool: "claude-code-config", launcher: "claude-code-config" },
  { tool: "fda-launcher", launcher: "install" },
];

for (const { tool, launcher } of toolLaunchers) {
  const crateLauncher = `crates/${tool}/${launcher}`;
  const toolDir = `tools/${tool}`;
  const toolLauncher = `${toolDir}/${launcher}`;
  if (!dryRun) {
    mkdirSync(join(root, toolDir), { recursive: true });
  }
  run(`git mv ${crateLauncher} ${toolLauncher}`);
}

// 3. Rewrite relative paths in Cargo.toml files under crates/
console.log("\n--- Rewriting crates/*/Cargo.toml dependency paths ---");
const cratesDir = join(root, "crates");
const crateEntries = dryRun
  ? moves.filter((m) => m.kind === "crate").map((m) => m.to.replace(/^crates\//, ""))
  : (existsSync(cratesDir) ? readdirSync(cratesDir) : []);

for (const c of crateEntries) {
  const cargoPath = join(cratesDir, c, "Cargo.toml");
  if (!existsSync(cargoPath)) continue;

  let content = readFileSync(cargoPath, "utf8");
  let changed = false;

  // Rewrite ../../libs/agent to ../sjel-agent
  if (content.includes("../../libs/agent")) {
    content = content.replaceAll("../../libs/agent", "../sjel-agent");
    changed = true;
  }
  // Rewrite ../../libs/ to ../
  if (content.includes("../../libs/")) {
    content = content.replaceAll("../../libs/", "../");
    changed = true;
  }
  // Rewrite ../../capabilities/backup to ../backup
  if (content.includes("../../capabilities/backup")) {
    content = content.replaceAll("../../capabilities/backup", "../backup");
    changed = true;
  }
  // Rewrite ../../capabilities/ to ../
  if (content.includes("../../capabilities/")) {
    content = content.replaceAll("../../capabilities/", "../");
    changed = true;
  }
  // Rewrite ../../tools/ to ../
  if (content.includes("../../tools/")) {
    content = content.replaceAll("../../tools/", "../");
    changed = true;
  }
  // Rewrite ../agent to ../sjel-agent (for former libs referring to agent lib)
  const lines = content.split("\n");
  const newLines = lines.map((line) => {
    if (line.includes('path = "../agent"') || line.includes('path="../agent"')) {
      changed = true;
      return line.replace('"../agent"', '"../sjel-agent"');
    }
    return line;
  });
  content = newLines.join("\n");

  if (changed) {
    console.log(`Rewrote paths in crates/${c}/Cargo.toml`);
    if (!dryRun) {
      writeFileSync(cargoPath, content, "utf8");
    }
  }
}

// 4. Rewrite paths in apps/gpui/
console.log("\n--- Rewriting apps/gpui Cargo.toml files ---");
const gpuiCargoFiles = [
  join(root, "apps/gpui/core/Cargo.toml"),
  join(root, "apps/gpui/ios/Cargo.toml"),
  join(root, "apps/gpui/Cargo.toml"),
  join(root, "apps/gpui/ui/Cargo.toml"),
];

for (const p of gpuiCargoFiles) {
  if (!existsSync(p)) continue;
  let content = readFileSync(p, "utf8");
  let changed = false;
  if (content.includes("../../../libs/")) {
    content = content.replaceAll("../../../libs/", "../../../crates/");
    changed = true;
  }
  if (changed) {
    console.log(`Rewrote paths in ${p}`);
    if (!dryRun) {
      writeFileSync(p, content, "utf8");
    }
  }
}

// 5. Update root Cargo.toml members = ["crates/*"]
console.log("\n--- Updating root Cargo.toml ---");
const rootCargoPath = join(root, "Cargo.toml");
let rootCargo = readFileSync(rootCargoPath, "utf8");
rootCargo = rootCargo.replace(
  /members\s*=\s*\[[\s\S]*?\]/,
  'members = [\n    "crates/*",\n]'
);
if (!dryRun) {
  writeFileSync(rootCargoPath, rootCargo, "utf8");
}
console.log("Updated root Cargo.toml members to [\"crates/*\"]");

// 6. Update tools/lib/paths.sh
console.log("\n--- Updating tools/lib/paths.sh ---");
const pathsSh = join(root, "tools/lib/paths.sh");
replaceInFile(
  pathsSh,
  `sjel_manifest_for() {
  local name="$1" root_mf="" overlay_mf=""
  if [ -f "$SJEL_CAPS_DIR/$name/service.toml" ]; then
    root_mf="$SJEL_CAPS_DIR/$name/service.toml"
  fi`,
  `sjel_manifest_for() {
  local name="$1" root_mf="" overlay_mf=""
  if [ -f "$SJEL_ROOT/crates/$name/service.toml" ]; then
    root_mf="$SJEL_ROOT/crates/$name/service.toml"
  elif [ -f "$SJEL_ROOT/services/$name/service.toml" ]; then
    root_mf="$SJEL_ROOT/services/$name/service.toml"
  elif [ -f "$SJEL_CAPS_DIR/$name/service.toml" ]; then
    root_mf="$SJEL_CAPS_DIR/$name/service.toml"
  fi`
);

// 6b. Update tools/lib/sjel-cli.sh
console.log("\n--- Updating tools/lib/sjel-cli.sh ---");
const sjelCliSh = join(root, "tools/lib/sjel-cli.sh");
replaceInFile(
  sjelCliSh,
  `    local crate
    crate="$(cd "$(dirname "\${BASH_SOURCE[0]:-\$0}")/../sjel-cli" 2>/dev/null && pwd)" || crate=""
    if [ -z "$crate" ] || [ ! -f "$crate/Cargo.toml" ]; then
      echo "$sub: no tools/sjel-cli crate beside this launcher and SJEL_CLI_BIN is unset" >&2
      exit 2
    fi`,
  `    local crate
    crate="$(cd "$(dirname "\${BASH_SOURCE[0]:-\$0}")/../../crates/sjel-cli" 2>/dev/null && pwd)" || crate=""
    if [ -z "$crate" ] || [ ! -f "$crate/Cargo.toml" ]; then
      crate="$(cd "$(dirname "\${BASH_SOURCE[0]:-\$0}")/../sjel-cli" 2>/dev/null && pwd)" || crate=""
    fi
    if [ -z "$crate" ] || [ ! -f "$crate/Cargo.toml" ]; then
      echo "$sub: no sjel-cli crate found and SJEL_CLI_BIN is unset" >&2
      exit 2
    fi`
);

// 6c. Update tool launchers for relocated crates
console.log("\n--- Updating tool launcher crate paths ---");
replaceInFile(
  join(root, "tools/storage/storage"),
  `_stale=""
if [ ! -x "$BIN" ]; then
  _stale="yes"
elif [ -n "$(find "$_here/src" "$_here/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  _stale="yes"
fi`,
  `_crate="$SJEL_ROOT/crates/storage"
_stale=""
if [ ! -x "$BIN" ]; then
  _stale="yes"
elif [ -n "$(find "$_crate/src" "$_crate/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  _stale="yes"
fi`
);

replaceInFile(
  join(root, "tools/sjel-mcp/sjel-mcp"),
  `_stale=""
if [ ! -x "$BIN" ]; then
  _stale="yes"
elif [ -n "$(find "$_here/src" "$_here/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  _stale="yes"
fi`,
  `_crate="$SJEL_ROOT/crates/sjel-mcp"
_stale=""
if [ ! -x "$BIN" ]; then
  _stale="yes"
elif [ -n "$(find "$_crate/src" "$_crate/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  _stale="yes"
fi`
);

replaceInFile(
  join(root, "tools/capability-auth/capability-auth"),
  `_stale=""
if [ ! -x "$BIN" ]; then
  _stale="yes"
elif [ -n "$(find "$_here/src" "$_here/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  _stale="yes"
fi`,
  `_crate="$SJEL_ROOT/crates/capability-auth"
_stale=""
if [ ! -x "$BIN" ]; then
  _stale="yes"
elif [ -n "$(find "$_crate/src" "$_crate/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  _stale="yes"
fi`
);

replaceInFile(
  join(root, "tools/claude-code-config/claude-code-config"),
  `_stale=""
if [ ! -x "$BIN" ]; then
  _stale="yes"
elif [ -n "$(find "$_here/src" "$_here/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  _stale="yes"
fi`,
  `_crate="$SJEL_ROOT/crates/claude-code-config"
_stale=""
if [ ! -x "$BIN" ]; then
  _stale="yes"
elif [ -n "$(find "$_crate/src" "$_crate/Cargo.toml" -newer "$BIN" -print -quit 2>/dev/null)" ]; then
  _stale="yes"
fi`
);

// 7. Update crates/sjel-cli/src/paths.rs
console.log("\n--- Updating sjel-cli paths.rs ---");
const sjelCliPathsRs = existsSync(join(root, "crates/sjel-cli/src/paths.rs"))
  ? join(root, "crates/sjel-cli/src/paths.rs")
  : join(root, "tools/sjel-cli/src/paths.rs");

replaceInFile(
  sjelCliPathsRs,
  `    pub fn manifest(&self, name: &str) -> Manifest {
        let core = self.caps_dir.join(name).join("service.toml");`,
  `    pub fn manifest(&self, name: &str) -> Manifest {
        let crates_mf = self.root.join("crates").join(name).join("service.toml");
        let services_mf = self.root.join("services").join(name).join("service.toml");
        let core = if crates_mf.is_file() {
            crates_mf
        } else if services_mf.is_file() {
            services_mf
        } else {
            self.caps_dir.join(name).join("service.toml")
        };`
);

replaceInFile(
  sjelCliPathsRs,
  `    pub fn cap_dir(&self, name: &str) -> Option<PathBuf> {
        let core = self.caps_dir.join(name);
        if core.is_dir() {
            return Some(core);
        }`,
  `    pub fn cap_dir(&self, name: &str) -> Option<PathBuf> {
        let crates_dir = self.root.join("crates").join(name);
        if crates_dir.is_dir() {
            return Some(crates_dir);
        }
        let services_dir = self.root.join("services").join(name);
        if services_dir.is_dir() {
            return Some(services_dir);
        }
        let core = self.caps_dir.join(name);
        if core.is_dir() {
            return Some(core);
        }`
);

// 8. Update crates/sjel-cli/src/self_model/model.rs
console.log("\n--- Updating sjel-cli self_model/model.rs ---");
const selfModelRs = existsSync(join(root, "crates/sjel-cli/src/self_model/model.rs"))
  ? join(root, "crates/sjel-cli/src/self_model/model.rs")
  : join(root, "tools/sjel-cli/src/self_model/model.rs");

replaceInFile(
  selfModelRs,
  `    match root {
        "capabilities" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Capability,
        }),
        "libs" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Lib,
        }),`,
  `    match root {
        "crates" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Capability,
        }),
        "services" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Capability,
        }),
        "capabilities" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Capability,
        }),
        "libs" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Lib,
        }),`
);

// 9. Update crates/sjel-cli/src/self_model/mod.rs
console.log("\n--- Updating sjel-cli self_model/mod.rs ---");
const selfModRs = existsSync(join(root, "crates/sjel-cli/src/self_model/mod.rs"))
  ? join(root, "crates/sjel-cli/src/self_model/mod.rs")
  : join(root, "tools/sjel-cli/src/self_model/mod.rs");

replaceInFile(
  selfModRs,
  `        let name = match segments.as_slice() {
            ["capabilities", name, "service.toml"] => *name,
            [name, "service.toml"] => *name,
            _ => continue,
        };`,
  `        let name = match segments.as_slice() {
            ["crates", name, "service.toml"] => *name,
            ["services", name, "service.toml"] => *name,
            ["capabilities", name, "service.toml"] => *name,
            [name, "service.toml"] => *name,
            _ => continue,
        };`
);

replaceInFile(
  selfModRs,
  `    add_unit_dirs(
        root,
        "capabilities",
        UnitKind::Capability,
        &mut kind_by_unit,
    );
    add_unit_dirs(root, "libs", UnitKind::Lib, &mut kind_by_unit);`,
  `    add_unit_dirs(root, "crates", UnitKind::Lib, &mut kind_by_unit);
    add_unit_dirs(root, "services", UnitKind::Capability, &mut kind_by_unit);
    for name in declared.keys() {
        if kind_by_unit.contains_key(name) {
            kind_by_unit.insert(name.clone(), UnitKind::Capability);
        }
    }
    add_unit_dirs(
        root,
        "capabilities",
        UnitKind::Capability,
        &mut kind_by_unit,
    );
    add_unit_dirs(root, "libs", UnitKind::Lib, &mut kind_by_unit);`
);

// 10. Update crates/sjel-cli/src/doctor/checks.rs
console.log("\n--- Updating sjel-cli doctor/checks.rs ---");
const doctorChecksRs = existsSync(join(root, "crates/sjel-cli/src/doctor/checks.rs"))
  ? join(root, "crates/sjel-cli/src/doctor/checks.rs")
  : join(root, "tools/sjel-cli/src/doctor/checks.rs");

replaceInFile(
  doctorChecksRs,
  `        let root_dir = ctx.root.join("capabilities").join(name);`,
  `        let root_dir = if ctx.root.join("crates").join(name).exists() {
            ctx.root.join("crates").join(name)
        } else if ctx.root.join("services").join(name).exists() {
            ctx.root.join("services").join(name)
        } else {
            ctx.root.join("capabilities").join(name)
        };`
);

replaceInFile(
  doctorChecksRs,
  `        let managed = ctx.root.join("capabilities").join(cap).exists()
            || overlay_dir(ctx).join("capabilities").join(cap).exists();`,
  `        let managed = ctx.root.join("crates").join(cap).exists()
            || ctx.root.join("services").join(cap).exists()
            || ctx.root.join("capabilities").join(cap).exists()
            || overlay_dir(ctx).join("capabilities").join(cap).exists();`
);

replaceInFile(
  doctorChecksRs,
  `        (ctx.root.join("capabilities"), "capabilities"),`,
  `        (ctx.root.join("crates"), "crates"),
        (ctx.root.join("services"), "services"),
        (ctx.root.join("capabilities"), "capabilities"),`
);

replaceInFile(
  doctorChecksRs,
  `        (ctx.root.join("capabilities"), ""),`,
  `        (ctx.root.join("crates"), ""),
        (ctx.root.join("services"), ""),
        (ctx.root.join("capabilities"), ""),`
);

// 11. Update tools/check-service-tomls.sh
console.log("\n--- Updating tools/check-service-tomls.sh ---");
const checkServiceTomls = join(root, "tools/check-service-tomls.sh");
replaceInFile(
  checkServiceTomls,
  `for svc in "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml`,
  `for svc in "$SJEL_ROOT"/crates/*/service.toml "$SJEL_ROOT"/services/*/service.toml "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml`
);

// 12. Update tools/generate-architecture.sh
console.log("\n--- Updating tools/generate-architecture.sh ---");
const genArch = join(root, "tools/generate-architecture.sh");
replaceInFile(
  genArch,
  `  for dir in "$SJEL_ROOT"/capabilities/*/; do`,
  `  for dir in "$SJEL_ROOT"/crates/*/ "$SJEL_ROOT"/services/*/ "$SJEL_ROOT"/capabilities/*/; do`
);
replaceInFile(
  genArch,
  `  for st in "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml; do`,
  `  for st in "$SJEL_ROOT"/crates/*/service.toml "$SJEL_ROOT"/services/*/service.toml "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml; do`
);

// 13. Update tools/check-generator-inputs-tracked.sh
console.log("\n--- Updating tools/check-generator-inputs-tracked.sh ---");
const checkGenInputs = join(root, "tools/check-generator-inputs-tracked.sh");
replaceInFile(
  checkGenInputs,
  `for f in "$SJEL_ROOT"/capabilities/*/README.md "$SJEL_ROOT"/capabilities/*/service.toml \\`,
  `for f in "$SJEL_ROOT"/crates/*/README.md "$SJEL_ROOT"/crates/*/service.toml "$SJEL_ROOT"/services/*/README.md "$SJEL_ROOT"/services/*/service.toml "$SJEL_ROOT"/capabilities/*/README.md "$SJEL_ROOT"/capabilities/*/service.toml \\`
);

// 14. Update tools/check-route-schemas.sh
console.log("\n--- Updating tools/check-route-schemas.sh ---");
const checkRouteSchemas = join(root, "tools/check-route-schemas.sh");
replaceInFile(
  checkRouteSchemas,
  `cap="$(printf '%s' "$f" | sed -E 's|.*/capabilities/([^/]+)/.*|\\1|')"`,
  `cap="$(printf '%s' "$f" | sed -E 's|.*/(capabilities|crates)/([^/]+)/.*|\\2|')"`
);
replaceInFile(
  checkRouteSchemas,
  `grep -rl 'const ROUTES' "$ROOT"/capabilities/*/src/*.rs 2>/dev/null | sort`,
  `grep -rl 'const ROUTES' "$ROOT"/crates/*/src/*.rs "$ROOT"/capabilities/*/src/*.rs 2>/dev/null | sort`
);

// 15. Update tools/check-manifest-integrity.sh
console.log("\n--- Updating tools/check-manifest-integrity.sh ---");
const checkManifestIntegrity = join(root, "tools/check-manifest-integrity.sh");
replaceInFile(
  checkManifestIntegrity,
  `for svc in "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml; do`,
  `for svc in "$SJEL_ROOT"/crates/*/service.toml "$SJEL_ROOT"/services/*/service.toml "$SJEL_ROOT"/capabilities/*/service.toml "$SJEL_ROOT"/*/service.toml; do`
);
replaceInFile(
  checkManifestIntegrity,
  `    if [ ! -d "$SJEL_ROOT/capabilities/$dep" ]; then
      echo "FAIL [$owner]: requires '$dep' but capabilities/$dep/ does not exist" >&2`,
  `    if [ ! -d "$SJEL_ROOT/crates/$dep" ] && [ ! -d "$SJEL_ROOT/services/$dep" ] && [ ! -d "$SJEL_ROOT/capabilities/$dep" ]; then
      echo "FAIL [$owner]: requires '$dep' but neither crates/$dep/, services/$dep/, nor capabilities/$dep/ exists" >&2`
);

// 16. Update tools/check-bun-install-policy.sh
console.log("\n--- Updating tools/check-bun-install-policy.sh ---");
const checkBun = join(root, "tools/check-bun-install-policy.sh");
replaceInFile(
  checkBun,
  `require "capabilities/comms/clipper/README.md" "bun install --frozen-lockfile --ignore-scripts"`,
  `require "crates/comms/clipper/README.md" "bun install --frozen-lockfile --ignore-scripts"`
);

// 17. Update website/scripts/routes.ts & reference.ts
console.log("\n--- Updating website scripts ---");
const webRoutesTs = join(root, "website/scripts/routes.ts");
replaceInFile(
  webRoutesTs,
  `  const src = join(root, "capabilities", name, "src");`,
  `  const src = existsSync(join(root, "crates", name, "src"))
    ? join(root, "crates", name, "src")
    : join(root, "capabilities", name, "src");`
);

const webRefTs = join(root, "website/scripts/reference.ts");
replaceInFile(
  webRefTs,
  `    case "capability": return \`capabilities/\${unit.name}\`;`,
  `    case "capability": return existsSync(join(root, "crates", unit.name)) ? \`crates/\${unit.name}\` : (existsSync(join(root, "services", unit.name)) ? \`services/\${unit.name}\` : \`capabilities/\${unit.name}\`);`
);

// 18. Update .github/workflows/security.yml
console.log("\n--- Updating .github/workflows/security.yml ---");
const secYml = join(root, ".github/workflows/security.yml");
replaceInFile(
  secYml,
  `for m in capabilities/*/service.toml; do`,
  `for m in crates/*/service.toml services/*/service.toml capabilities/*/service.toml; do`
);

// 19. Update CONTRIBUTING.md
console.log("\n--- Updating CONTRIBUTING.md ---");
const contribMd = join(root, "CONTRIBUTING.md");
replaceInFile(
  contribMd,
  `- **Spine**: The core runtime and developer surface: root manifests, \`schemas/\`, \`tools/\`, \`libs/\`,
  the \`sjel-status\` shell and the apps in \`apps/\`.
- **Capability**: Autonomous domain units under \`capabilities/<name>/\`, declared via \`service.toml\`.
- **Pack**: Modular agent know-how and workflows under \`Packs/<name>/\`.

See [website/content/architecture/index.md](website/content/architecture/index.md) for full topology and boundary rules.
Shared utilities belong in \`libs/\`; contracts belong in \`schemas/\`; operator tools belong in \`tools/\`.`,
  `- **Spine**: The core runtime and developer surface: root manifests, \`schemas/\`, \`tools/\` (scripts),
  \`crates/\` (shared crates, apps and tools), the \`sjel-status\` gateway and the apps in \`apps/\`.
- **Capability**: Autonomous domain units declared via \`service.toml\` (Rust crates under \`crates/<name>/\`,
  supervised third-party software and scripts under \`services/<name>/\`).
- **Pack**: Modular agent know-how and workflows under \`Packs/<name>/\`.

See [website/content/architecture/index.md](website/content/architecture/index.md) for full topology and boundary rules.
Shared Rust code belongs in \`crates/\`; third-party services in \`services/\`; contracts belong in \`schemas/\`; operator tools belong in \`tools/\`.`
);

console.log("\nMigration completed successfully.");
