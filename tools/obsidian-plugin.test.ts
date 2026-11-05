// tools/obsidian-plugin.test.ts — each built and installed Pack Obsidian plugin matches its src/.
// dist/ is gitignored and the vault copy is machine state, so each check skips where its file
// does not exist; a skip means "not built here", not "verified".
import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const PACKS = join(import.meta.dir, "..", "Packs");

/** Every Pack plugin that `tools/obsidian-plugin` builds: the ones with src/main.ts. */
const plugins = readdirSync(PACKS).flatMap((pack) => {
	const dir = join(PACKS, pack, "plugins");
	if (!existsSync(dir)) return [];
	return readdirSync(dir)
		.filter((name) => existsSync(join(dir, name, "src", "main.ts")))
		.map((name) => ({ name, root: join(dir, name) }));
});

function srcHash(root: string): string {
	const files = readdirSync(join(root, "src"), { recursive: true })
		.map(String)
		.filter((f) => f.endsWith(".ts"))
		.map((f) => `src/${f}`)
		.sort();
	const hash = createHash("sha256");
	for (const f of files) hash.update(readFileSync(join(root, f)));
	return hash.digest("hex");
}

/** The vault root from the overlay's `vault_root`; null when this machine has none. */
function vaultRoot(): string | null {
	const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT;
	if (!overlay) return null;
	const config = join(overlay, "config", "knowledge.toml");
	if (!existsSync(config)) return null;
	const root = /^vault_root\s*=\s*"([^"]+)"/m.exec(readFileSync(config, "utf8"))?.[1];
	return root ? root.replace(/^~(?=\/|$)/, homedir()) : null;
}

const stamp = (file: string) => readFileSync(file, "utf8").split("\n", 1)[0];
const vault = vaultRoot();

test("the build tool finds both locally developed plugins", () => {
	expect(plugins.map((p) => p.name)).toEqual(expect.arrayContaining(["interactive", "pi-learn"]));
});

for (const { name, root } of plugins) {
	const dist = join(root, "dist", "main.js");
	const installed = vault ? join(vault, ".obsidian", "plugins", name, "main.js") : null;
	const built = existsSync(dist) && stamp(dist).startsWith("// src-sha256:");

	describe(`${name} plugin build (rebuild with tools/obsidian-plugin ${name} install)`, () => {
		test.skipIf(!built)("dist/main.js matches src", () => {
			expect(stamp(dist)).toBe(`// src-sha256: ${srcHash(root)}`);
		});

		test.skipIf(installed === null || !existsSync(installed))("the vault's installed copy matches src", () => {
			expect(stamp(installed!)).toBe(`// src-sha256: ${srcHash(root)}`);
		});
	});
}
