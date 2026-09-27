#!/usr/bin/env bun
/**
 * obsidian-deploy — materialize the plugins Obsidian's own UI cannot install.
 *
 * Scope, narrowed 2026-08-18: this tool owns ONLY plugins with no installable upstream —
 * own code, forks, and builds newer than any tag. Everything on the community registry
 * is installed and updated through Obsidian's UI, which gives update notifications,
 * changelogs and mobile installs that a pinned TOML never will. The vault repo already
 * tracks each plugin's main.js under .obsidian, so the plugin set is reproducible from a vault
 * clone with or without this tool; declaring public plugins here only created a second
 * source of truth for a fact the vault already carried.
 *
 * The overlay's config/obsidian-plugins.toml declares that narrow set. Each plugin is
 * `overlay` (artifacts tracked in the overlay) or `upstream` (pinned by repo + version,
 * fetched from that GitHub release). The upstream path is kept for the case it was built
 * for and still serves: a plugin currently running a build newer than its newest tag,
 * which flips back to a pin the moment upstream catches up. Deploy copies manifest.json +
 * main.js (+ styles.css) into the vault's .obsidian/plugins/ as real files, so plugins
 * reach mobile through the vault's own git repo. Per-vault settings (data.json) are
 * preserved — rescued from a symlink's target before the link goes, never overwritten
 * after.
 *
 * Pruning is deliberately confined to ids this file declares. A plugin the tool never
 * declared is a UI install, and deleting it is not this tool's business.
 *
 * Zero hardcoded paths (CONTRIBUTING.md#one-manifest-per-concern): overlay from
 * axon.local.toml/axon.toml, vault from this machine's machine.toml [[state_mount]]
 * with data_class = "vault", plugin set from the overlay manifest.
 *
 * Usage:
 *   bun tools/obsidian-deploy.ts             # fetch what's missing, deploy, prune
 *   bun tools/obsidian-deploy.ts --dry-run   # report only: no downloads, no writes
 *   bun tools/obsidian-deploy.ts --refetch   # re-download every upstream plugin
 */
import { existsSync, lstatSync, readlinkSync, readdirSync, rmSync, mkdirSync, copyFileSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { homedir } from "node:os";
import { resolveMachineToml, resolveOverlayRoot } from "./lib/overlay.ts";

type Plugin = {
  id: string;
  repo?: string;
  version?: string;
  source?: "upstream" | "overlay";
  active?: boolean;
  note?: string;
};

const DRY = process.argv.includes("--dry-run");
const REFETCH = process.argv.includes("--refetch");
const home = homedir();
const expand = (p: string) => resolve(p.replace(/^~(?=\/|$)/, home));

const overlayResolution = resolveOverlayRoot();
if (!overlayResolution) throw new Error("no 'overlay' in axon.local.toml or axon.toml — run tools/install.sh");
const overlay = overlayResolution.root;

// State mounts describe a machine, not the platform, so they live in the overlay's
// machine manifest — selected the same way every other tool selects it.
const machine = resolveMachineToml(overlay);
if (!machine || !existsSync(machine.path)) throw new Error(`no machine manifest at ${machine?.path ?? `${overlay}/config`}`);
const machineToml = Bun.TOML.parse(await Bun.file(machine.path).text()) as any;
const vaultMount = (machineToml.state_mount ?? []).find((m: any) => m.data_class === "vault");
if (!vaultMount) throw new Error(`${machine.path}: no [[state_mount]] with data_class="vault"`);
const vault = expand(vaultMount.path);

const cfgPath = join(overlay, "config", "obsidian-plugins.toml");
if (!existsSync(cfgPath)) throw new Error(`missing overlay config: ${cfgPath}`);
const cfg = Bun.TOML.parse(await Bun.file(cfgPath).text()) as { source_dir?: string; plugin?: Plugin[] };
if (!cfg.source_dir) throw new Error(`${cfgPath}: no source_dir`);
const sourceDir = expand(cfg.source_dir);
const declared = cfg.plugin ?? [];
if (declared.length === 0) throw new Error(`${cfgPath}: no [[plugin]] entries`);
const active = declared.filter((p) => p.active !== false);

const pluginsDir = join(vault, ".obsidian", "plugins");
// styles.css is optional: plenty of plugins ship none, and a 404 for it is not a failure.
const ARTIFACTS: { name: string; required: boolean }[] = [
  { name: "manifest.json", required: true },
  { name: "main.js", required: true },
  { name: "styles.css", required: false },
];

/**
 * Pull one plugin's release artifacts into its source dir.
 *
 * Obsidian plugins tag releases either bare ("1.5.10") or v-prefixed, with no way to
 * tell from the manifest which convention a repo follows, so both are tried. The
 * fetched manifest's version is checked against the pin: a tag that resolves to the
 * wrong build is exactly the failure a pin exists to prevent.
 */
async function fetchUpstream(p: Plugin, dir: string): Promise<void> {
  if (!p.repo || !p.version) throw new Error(`${p.id}: an upstream plugin needs both repo and version`);
  const errors: string[] = [];
  for (const tag of [p.version, `v${p.version}`]) {
    const base = `https://github.com/${p.repo}/releases/download/${tag}`;
    const files: { name: string; body: ArrayBuffer }[] = [];
    let ok = true;
    for (const artifact of ARTIFACTS) {
      const res = await fetch(`${base}/${artifact.name}`);
      if (res.ok) files.push({ name: artifact.name, body: await res.arrayBuffer() });
      else if (artifact.required) { ok = false; errors.push(`${tag}/${artifact.name}: HTTP ${res.status}`); break; }
    }
    if (!ok) continue;

    // Some plugins keep styles.css in the repo and never attach it to the release —
    // Obsidian's own installer then ships without it, but a vault that already has the
    // file would silently lose styling on the next deploy. Fall back to the tag's tree.
    if (!files.some((f) => f.name === "styles.css")) {
      const raw = await fetch(`https://raw.githubusercontent.com/${p.repo}/${tag}/styles.css`);
      if (raw.ok) files.push({ name: "styles.css", body: await raw.arrayBuffer() });
    }

    const fetched = JSON.parse(new TextDecoder().decode(files.find((f) => f.name === "manifest.json")!.body));
    if (fetched.version !== p.version) {
      errors.push(`${tag}: release manifest says ${fetched.version}, manifest pins ${p.version}`);
      continue;
    }
    mkdirSync(dir, { recursive: true });
    // Written only after every required artifact is in hand, so a mid-download failure
    // cannot leave a half-updated plugin behind.
    for (const f of files) await Bun.write(join(dir, f.name), f.body);
    // A styles.css left over from an older version would otherwise be deployed forever.
    const stale = join(dir, "styles.css");
    if (!files.some((f) => f.name === "styles.css") && existsSync(stale)) rmSync(stale);
    console.log(`  ↓ ${p.id} ${p.version} from ${p.repo} (${tag})`);
    return;
  }
  throw new Error(`${p.id}: no release assets for ${p.version} — ${errors.join("; ")}`);
}

let deployed = 0;
const desktopOnly: string[] = [], failed: string[] = [], rescued: string[] = [], fetchedIds: string[] = [];

for (const plugin of active) {
  const src = join(sourceDir, plugin.id);
  const dest = join(pluginsDir, plugin.id);

  if ((plugin.source ?? "upstream") === "upstream") {
    let stale: boolean;
    try {
      const manifestPath = join(src, "manifest.json");
      const localVersion = existsSync(manifestPath) ? JSON.parse(await Bun.file(manifestPath).text()).version : null;
      stale = REFETCH || localVersion !== plugin.version || !existsSync(join(src, "main.js"));
    } catch {
      stale = true;
    }
    if (stale) {
      if (DRY) { console.log(`  [dry-run] would fetch ${plugin.id} ${plugin.version}`); }
      else {
        try { await fetchUpstream(plugin, src); fetchedIds.push(plugin.id); }
        catch (err) { failed.push(plugin.id); console.error(`✗ ${(err as Error).message}`); continue; }
      }
    }
  }

  if (!existsSync(join(src, "manifest.json")) || !existsSync(join(src, "main.js"))) {
    failed.push(plugin.id);
    const why = (plugin.source ?? "upstream") === "overlay"
      ? "overlay-sourced plugin with no artifacts — the overlay is supposed to carry them"
      : "missing manifest.json/main.js";
    console.error(`✗ ${plugin.id}: ${why} under ${src}`);
    continue;
  }
  const manifest = JSON.parse(await Bun.file(join(src, "manifest.json")).text());
  if (manifest.isDesktopOnly) desktopOnly.push(plugin.id);

  // Rescue settings if dest is a symlink (settings lived through the link).
  let rescuedData: string | null = null;
  if (existsSync(dest) && lstatSync(dest).isSymbolicLink()) {
    const target = resolve(dirname(dest), readlinkSync(dest));
    const d = join(target, "data.json");
    if (existsSync(d)) { rescuedData = await Bun.file(d).text(); rescued.push(plugin.id); }
    if (!DRY) rmSync(dest);
  }
  if (!DRY) {
    mkdirSync(dest, { recursive: true });
    for (const artifact of ARTIFACTS) {
      const f = join(src, artifact.name);
      if (existsSync(f)) copyFileSync(f, join(dest, artifact.name));
    }
    const destData = join(dest, "data.json");
    if (rescuedData !== null && !existsSync(destData)) await Bun.write(destData, rescuedData);
  }
  deployed++;
  console.log(`✓ ${plugin.id}${manifest.isDesktopOnly ? " (desktop-only)" : ""}${rescuedData ? " [settings rescued]" : ""}`);
}

// Prune only what this file declares and then deactivates. Everything else in the
// plugins dir is a UI install the tool does not own; the old sweep of "anything not in
// the active set" silently deleted those on the next run.
const inactiveIds = declared.filter((p) => p.active === false).map((p) => p.id);
const pruned: string[] = [];
for (const id of inactiveIds) {
  if (!existsSync(join(pluginsDir, id))) continue;
  pruned.push(id);
  if (!DRY) rmSync(join(pluginsDir, id), { recursive: true });
}

// Enable-state check: community-plugins.json is owned by Obsidian's UI, but an enabled
// id with no directory at all produces "Failed to load" on every startup. Only a missing
// directory is the fault — an enabled id this file never declared is the normal case now.
const cpPath = join(vault, ".obsidian", "community-plugins.json");
if (existsSync(cpPath)) {
  const enabled: string[] = JSON.parse(await Bun.file(cpPath).text());
  const orphans = enabled.filter((id) => !existsSync(join(pluginsDir, id)));
  if (orphans.length) console.warn(`⚠ enabled but no plugin directory (will fail to load): ${orphans.join(", ")}`);
}

console.log(
  `\n${DRY ? "[dry-run] " : ""}deployed ${deployed}/${active.length}` +
  ` · fetched ${fetchedIds.length} · rescued settings ${rescued.length}` +
  ` · pruned ${pruned.length}${pruned.length ? ` (${pruned.join(", ")})` : ""}`,
);
if (desktopOnly.length) console.log(`desktop-only (won't enable on mobile): ${desktopOnly.join(", ")}`);
if (failed.length) { console.error(`FAILED: ${failed.join(", ")}`); process.exit(1); }
