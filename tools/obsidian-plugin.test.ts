// tools/interactive-plugin.test.ts — the built and the installed interactive Obsidian plugin match
// the current src/. dist/ is gitignored and the vault copy is machine state, so each check skips
// where its file does not exist; a skip means "not built here", not "verified".
import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const PLUGIN = join(import.meta.dir, "..", "Packs", "knowledge", "plugins", "interactive");
const DIST = join(PLUGIN, "dist", "main.js");

function srcHash(): string {
	const files = readdirSync(join(PLUGIN, "src"), { recursive: true })
		.map(String)
		.filter((f) => f.endsWith(".ts"))
		.map((f) => `src/${f}`)
		.sort();
	const hash = createHash("sha256");
	for (const f of files) hash.update(readFileSync(join(PLUGIN, f)));
	return hash.digest("hex");
}

/** The installed copy, from the overlay's `vault_root`; null when this machine has none. */
function installedMain(): string | null {
	const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT;
	if (!overlay) return null;
	const config = join(overlay, "config", "knowledge.toml");
	if (!existsSync(config)) return null;
	const root = /^vault_root\s*=\s*"([^"]+)"/m.exec(readFileSync(config, "utf8"))?.[1];
	if (!root) return null;
	const main = join(root.replace(/^~(?=\/|$)/, homedir()), ".obsidian", "plugins", "interactive", "main.js");
	return existsSync(main) ? main : null;
}

const stamp = (file: string) => readFileSync(file, "utf8").split("\n", 1)[0];
const installed = installedMain();

describe("interactive plugin build (rebuild with tools/interactive-plugin install)", () => {
	test.skipIf(!existsSync(DIST))("dist/main.js matches src", () => {
		expect(stamp(DIST)).toBe(`// src-sha256: ${srcHash()}`);
	});

	test.skipIf(installed === null)("the vault's installed copy matches src", () => {
		expect(stamp(installed!)).toBe(`// src-sha256: ${srcHash()}`);
	});
});
