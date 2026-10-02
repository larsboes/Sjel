#!/usr/bin/env node
/**
 * bw-key — resolve inference API keys from the Bitwarden/Vaultwarden vault.
 *
 * Invoked on demand, one process per call, with nothing but the key on stdout.
 * The pi extension `index.ts` points each provider's `apiKey` at
 * `!<abs path>/bw-key.mjs <slug>`, so pi runs this whenever it needs a key.
 *
 * Vault items are Secure Notes in the `Axon` folder named
 * `inference-<slug>-api-key`; the key is the note body. Legacy items keep the
 * name their owner gave them and are declared per slug in the catalog.
 *
 * Modes
 *   bw-key.mjs <slug>                 print the key (default)
 *   bw-key.mjs --check <slug>         exit 0 if the key resolves, 1 otherwise
 *   bw-key.mjs --manifest <spec>...   JSON map of which slugs have a vault item
 *   bw-key.mjs --status               JSON vault/session diagnostics
 *   bw-key.mjs --unlock               unlock via the Axon bw-unlock helper
 *   bw-key.mjs --forget [<slug>...]   drop cached keys (all slugs when omitted)
 *
 * `<spec>` is `slug` or `slug=Exact Item Name`.
 *
 * Two failure modes shape most of this file, both observed rather than assumed:
 * `bw` exits 0 with empty stdout when it cannot read the vault without a
 * prompt, and a stale session in the environment outlives the session cached by
 * `bwu` on disk. So every read is `--nointeraction`, empty output is a failure
 * however the exit code looks, and every session candidate is tried.
 *
 * Environment
 *   BW_SESSION         session key; the on-disk Axon cache is preferred over it
 *   PI_BW_TTL          seconds to cache a fetched key (default 60, 0 = never)
 *   PI_BW_BIN          bw binary (default `bw`)
 *   PI_BW_TIMEOUT_MS   per-`bw`-call timeout (default 8000)
 *   PI_BW_FOLDER       vault folder holding inference items (default `Axon`)
 *   PI_BW_CACHE_DIR    cache directory (default ~/.cache/pi-inference-keys)
 *   PI_BW_UNLOCK       path to the unlock helper (default: $AXON_ROOT/tools/bw-unlock)
 */

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

const BIN = process.env.PI_BW_BIN || "bw";
/**
 * Two budgets, because the two callers answer to different clocks.
 *
 * The key path runs as pi's `apiKey` command, and pi kills that at 10s, so it
 * has to fail first and say why. The diagnostic path (--manifest, --status) runs
 * from the extension, where nothing is waiting on a 10s deadline, and `bw` is
 * measurably slower under load: an 8s budget there was observed timing out on a
 * busy machine and reporting a perfectly usable vault as unreachable.
 */
const TIMEOUT_MS = Number(process.env.PI_BW_TIMEOUT_MS || 8000);
const ADMIN_TIMEOUT_MS = Number(process.env.PI_BW_ADMIN_TIMEOUT_MS || 30000);
const TTL_MS = Math.max(0, Number(process.env.PI_BW_TTL ?? 60)) * 1000;
const FOLDER_NAME = process.env.PI_BW_FOLDER || "Axon";

/**
 * Keys come from the login keychain when they are there, and from the vault when
 * they are not.
 *
 * The order is about cost, not preference. This file runs as pi's `apiKey`
 * command once per registered provider, inside startup, and the calls are serial:
 * `security find-generic-password -w` answers in ~10 ms where `bw get notes`
 * costs ~2.9 s (a Node CLI, reading a self-hosted vault over Tailscale). Eight
 * providers' worth of that difference is the entire wait before pi's first
 * prompt — measured 2026-09-17 at 20-36 s, and ~0.1 s with the keys in the
 * keychain.
 *
 * PI_KEYCHAIN=0 disables the keychain read. It exists to compare the two paths.
 */
const KEYCHAIN_BIN = process.env.PI_KEYCHAIN_BIN || "/usr/bin/security";
const KEYCHAIN_OFF = process.env.PI_KEYCHAIN === "0";
const KEYCHAIN_TIMEOUT_MS = Number(process.env.PI_KEYCHAIN_TIMEOUT_MS || 5000);

const cacheDir = () =>
	process.env.PI_BW_CACHE_DIR || join(process.env.XDG_CACHE_HOME || join(homedir(), ".cache"), "pi-inference-keys");

/** Exit codes are part of the interface: the extension reports them verbatim. */
const EXIT = {
	usage: 2,
	locked: 3,
	missing: 4,
	unavailable: 6,
};

class Fail extends Error {
	constructor(message, code) {
		super(message);
		this.code = code;
	}
}

function sessionFiles() {
	const root = process.env.XDG_CACHE_HOME || join(homedir(), ".cache");
	return [join(root, "axon", "bw-session")];
}

/**
 * Session candidates, most-trustworthy first. `bwu` writes the on-disk cache at
 * unlock time, so it is fresher than an environment variable exported by a shell
 * that started earlier; the variable only wins when no cache file exists.
 */
function sessionCandidates() {
	const seen = new Set();
	const out = [];
	for (const file of sessionFiles()) {
		try {
			const value = readFileSync(file, "utf8").trim();
			if (value && !seen.has(value)) {
				seen.add(value);
				out.push({ value, from: file });
			}
		} catch {}
	}
	const env = (process.env.BW_SESSION || "").trim();
	if (env && !seen.has(env)) out.push({ value: env, from: "BW_SESSION" });
	return out;
}

/** Run bw with no interactive fallback: stdin is closed, so it must answer or fail. */
function bw(args, session, timeoutMs = TIMEOUT_MS) {
	const full = [...args, "--nointeraction"];
	if (session) full.push("--session", session);
	const result = spawnSync(BIN, full, {
		encoding: "utf8",
		timeout: timeoutMs,
		stdio: ["ignore", "pipe", "pipe"],
		env: process.env,
	});
	if (result.error) {
		const why = result.error.code === "ETIMEDOUT" ? `timed out after ${timeoutMs}ms` : result.error.message;
		throw new Fail(`\`${BIN} ${args.join(" ")}\` ${why}`, EXIT.unavailable);
	}
	const stdout = (result.stdout || "").trim();
	const stderr = (result.stderr || "").trim();
	if (result.status !== 0) {
		const detail = stderr.split("\n").find((line) => line.trim()) || `exit ${result.status}`;
		// A locked vault is the one failure the caller can act on, so it gets its own code.
		if (/locked/i.test(detail)) throw new Fail(`vault is locked (${detail})`, EXIT.locked);
		throw new Fail(detail, EXIT.missing);
	}
	if (!stdout) {
		// bw exits 0 with no output when it silently could not read the item.
		const status = readStatus(session, Math.min(timeoutMs, ADMIN_TIMEOUT_MS));
		if (status.locked) throw new Fail("vault is locked", EXIT.locked);
		throw new Fail(`\`${BIN} ${args.join(" ")}\` returned nothing`, EXIT.missing);
	}
	return stdout;
}

function readStatus(session, timeoutMs = ADMIN_TIMEOUT_MS) {
	try {
		const raw = bw(["status"], session, timeoutMs);
		const parsed = JSON.parse(raw);
		return {
			status: parsed.status || "unknown",
			locked: parsed.status === "locked",
			unauthenticated: parsed.status === "unauthenticated",
			serverUrl: parsed.serverUrl,
			userEmail: parsed.userEmail,
		};
	} catch (error) {
		return { status: "unavailable", locked: false, unavailable: true, error: error.message };
	}
}

/** First session candidate that can actually read the vault. */
function workingSession(timeoutMs = TIMEOUT_MS) {
	const failures = [];
	for (const candidate of sessionCandidates()) {
		try {
			bw(["status"], candidate.value, timeoutMs);
			return candidate.value;
		} catch (error) {
			failures.push(`${candidate.from}: ${error.message}`);
		}
	}
	if (failures.length > 0) {
		const locked = failures.some((line) => /locked/i.test(line));
		throw new Fail(
			locked
				? "vault is locked; run `bwu` (or /bw-unlock in pi), then retry"
				: `no usable Bitwarden session (${failures.join("; ")})`,
			locked ? EXIT.locked : EXIT.missing,
		);
	}
	throw new Fail("no Bitwarden session; run `bwu` (or /bw-unlock in pi)", EXIT.missing);
}

/**
 * Items whose name is not unique get picked deterministically, in this order:
 * the one inside the inference folder, then the most recently revised. This
 * vault genuinely holds two copies of several legacy items (a Vaultwarden
 * duplication, both in a folder named "Personal"), and refusing to choose would
 * make those providers unusable. The choice is surfaced as `ambiguous` by
 * --manifest so /bw-status can say so rather than hide it.
 */
function pickItem(matches, folderId) {
	const inFolder = matches.filter((item) => folderId && item.folderId === folderId);
	const pool = inFolder.length > 0 ? inFolder : matches;
	return [...pool].sort((a, b) => (b.revisionDate ?? "").localeCompare(a.revisionDate ?? ""))[0];
}

function folderIdNamed(session, name, timeoutMs = TIMEOUT_MS) {
	try {
		const folders = JSON.parse(bw(["list", "folders"], session, timeoutMs));
		return folders.find((folder) => folder.name === name)?.id;
	} catch {
		// A vault whose folders cannot be listed still has usable items; the
		// folder only breaks a tie a later step can break anyway.
		return undefined;
	}
}

/** Search-based fallback for a name that `bw get notes` could not resolve by
 * itself, which is how a duplicate name announces itself. */
function resolveItem(name, session, timeoutMs = TIMEOUT_MS) {
	let items;
	try {
		items = JSON.parse(bw(["list", "items", "--search", name], session, timeoutMs));
	} catch (error) {
		throw new Fail(`could not search the vault for "${name}": ${error.message}`, EXIT.missing);
	}
	const exact = items.filter((item) => item.name === name);
	if (exact.length === 0) throw new Fail(`no vault item named "${name}"`, EXIT.missing);
	return { item: pickItem(exact, folderIdNamed(session, FOLDER_NAME, timeoutMs)), count: exact.length };
}

/**
 * The hot path: read one key, spending as few `bw` invocations as possible.
 *
 * `bw status` is deliberately not called first. It costs a full Node startup of
 * its own and is the call most likely to stall on a busy machine, and a wrong
 * session answers just as clearly by failing the read itself. Every session
 * candidate is tried so a stale environment variable cannot shadow the session
 * `bwu` cached on disk.
 */
function fetchKeyWithAnySession(itemName) {
	const candidates = sessionCandidates();
	if (candidates.length === 0) {
		throw new Fail("no Bitwarden session; run `bwu` (or /bw-unlock in pi)", EXIT.missing);
	}
	const failures = [];
	for (const candidate of candidates) {
		try {
			return { key: fetchKey(itemName, candidate.value), session: candidate.value, from: candidate.from };
		} catch (error) {
			failures.push(`${candidate.from}: ${error.message}`);
			// Only a session problem justifies asking the next candidate. An answer
			// about the item itself (absent, empty, ambiguous) came from a working
			// session and is final; retrying it only ends up blaming the session for
			// a missing vault item.
			const sessionProblem = error.code === EXIT.locked || error.code === EXIT.unavailable;
			if (!sessionProblem) throw error;
			// A timeout will not resolve by asking a second session the same question.
			if (error.code === EXIT.unavailable) break;
		}
	}
	const detail = failures.join("; ");
	const locked = failures.some((line) => /locked/i.test(line));
	throw new Fail(
		locked
			? `vault is locked (${detail}); run \`bwu\` or /bw-unlock, then retry`
			: `no usable Bitwarden session (${detail})`,
		locked ? EXIT.locked : EXIT.missing,
	);
}

/** The key body, with the empty/absent cases distinguished rather than merged. */
function fetchKey(itemName, session) {
	let value;
	try {
		value = bw(["get", "notes", itemName], session);
	} catch (error) {
		if (error.code === EXIT.locked) throw error;
		// "Not found" and "More than one result" both land here; an id settles both.
		value = bw(["get", "notes", resolveItem(itemName, session).item.id], session);
	}
	if (!value) throw new Fail(`vault item "${itemName}" is empty`, EXIT.missing);
	if (/\s/.test(value)) {
		throw new Fail(
			`vault item "${itemName}" holds ${value.split(/\s+/).length} whitespace-separated tokens; ` +
				"a Secure Note should contain only the key",
			EXIT.missing,
		);
	}
	return value;
}

/**
 * `inference-<slug>-api-key`, the same string the vault convention already uses,
 * so the slug alone determines both names and there is no mapping table to drift.
 * The three legacy vault names ("Deepseek API Key", "Openrouter API Key",
 * "Kimi K2 API Token") are reached through their slug and are not carried into
 * the keychain, where the name would be wrong rather than merely old.
 */
function keychainService(slug) {
	return `inference-${slug}-api-key`;
}

/**
 * The key from the login keychain, or undefined.
 *
 * Absence is the ordinary case — nothing is migrated until it is — so a miss must
 * not throw and must not be reported as a failure. This mirrors the vault path's
 * own refusal of a note holding several tokens: a key is one token, and anything
 * else is a mistake worth surfacing rather than sending as an Authorization
 * header.
 */
function fetchKeychain(slug) {
	if (KEYCHAIN_OFF) return undefined;
	const result = spawnSync(KEYCHAIN_BIN, ["find-generic-password", "-s", keychainService(slug), "-w"], {
		encoding: "utf8",
		timeout: KEYCHAIN_TIMEOUT_MS,
		stdio: ["ignore", "pipe", "pipe"],
		env: process.env,
	});
	if (result.error || result.status !== 0) return undefined;
	const value = (result.stdout || "").replace(/\n$/, "");
	return value && !/\s/.test(value) ? value : undefined;
}

function cacheFile(slug) {
	return join(cacheDir(), `${slug}.json`);
}

function readCache(slug) {
	if (TTL_MS === 0) return undefined;
	try {
		const entry = JSON.parse(readFileSync(cacheFile(slug), "utf8"));
		if (typeof entry.key === "string" && Date.now() - entry.fetchedAt < TTL_MS) return entry;
	} catch {}
	return undefined;
}

function writeCache(slug, entry) {
	try {
		mkdirSync(cacheDir(), { recursive: true, mode: 0o700 });
		const file = cacheFile(slug);
		// Create with the right mode before the bytes exist, so the key is never
		// briefly readable by other local users.
		writeFileSync(file, "", { mode: 0o600 });
		writeFileSync(file, JSON.stringify(entry), { mode: 0o600 });
	} catch {}
}

function noteError(slug, message) {
	try {
		mkdirSync(cacheDir(), { recursive: true, mode: 0o700 });
		writeFileSync(
			join(cacheDir(), "last-error.json"),
			JSON.stringify({ slug, message, at: new Date().toISOString() }),
			{ mode: 0o600 },
		);
	} catch {}
}

function forget(slugs) {
	const dir = cacheDir();
	try {
		for (const file of readdirSync(dir)) {
			if (!file.endsWith(".json")) continue;
			if (slugs.length > 0 && !slugs.some((slug) => file === `${slug}.json`)) continue;
			rmSync(join(dir, file), { force: true });
		}
	} catch {}
}

function unlockHelper() {
	if (process.env.PI_BW_UNLOCK) return process.env.PI_BW_UNLOCK;
	const roots = [process.env.AXON_ROOT, join(homedir(), "Developer", "Axon")].filter(Boolean);
	for (const root of roots) {
		const candidate = join(root, "tools", "bw-unlock");
		if (existsSync(candidate)) return candidate;
	}
	return undefined;
}

function parseSpec(spec) {
	const eq = spec.indexOf("=");
	if (eq === -1) return { slug: spec, item: `inference-${spec}-api-key` };
	return { slug: spec.slice(0, eq), item: spec.slice(eq + 1) };
}

function printStatus() {
	const sessions = sessionCandidates().map((candidate) => candidate.from);
	let vault;
	let session;
	let failure;
	try {
		session = workingSession(ADMIN_TIMEOUT_MS);
		vault = readStatus(session);
	} catch (error) {
		vault = vault ?? readStatus(undefined, ADMIN_TIMEOUT_MS);
		failure = { code: error.code, message: error.message };
	}
	const payload = {
		vault: vault ?? { status: "unknown" },
		sessionCandidates: sessions,
		sessionUsable: Boolean(session),
		ttlSeconds: TTL_MS / 1000,
		cacheDir: cacheDir(),
		unlockHelper: unlockHelper(),
		failure,
	};
	process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`);
}

/**
 * Which catalog items exist.
 *
 * A listing that could not be read exits non-zero even though it still prints a
 * diagnostic object. The caller caches this, and an empty-but-successful answer
 * would look like "this vault has no keys", replacing a good cached inventory
 * with nothing and emptying the model picker for as long as the cache lives.
 */
function printManifest(specs) {
	const result = { checkedAt: new Date().toISOString(), vault: "unavailable", folder: FOLDER_NAME, synced: false, items: {} };
	let session;
	try {
		session = workingSession(ADMIN_TIMEOUT_MS);
	} catch (error) {
		result.vault = error.code === EXIT.locked ? "locked" : "unavailable";
		result.error = error.message;
		process.stdout.write(`${JSON.stringify(result)}\n`);
		process.exitCode = error.code ?? EXIT.unavailable;
		return;
	}
	// Pull remote changes before answering. The CLI reads a local cache, so a key
	// added from the phone or the web vault is invisible here until a sync, which
	// looks exactly like a missing item. Best-effort: an offline machine still
	// reports the local inventory rather than nothing.
	try {
		bw(["sync"], session, ADMIN_TIMEOUT_MS);
		result.synced = true;
	} catch {}
	const vault = readStatus(session);
	result.vault = vault.status;
	let items;
	try {
		items = JSON.parse(bw(["list", "items"], session, ADMIN_TIMEOUT_MS));
	} catch (error) {
		result.vault = "unavailable";
		result.error = error.message;
		process.stdout.write(`${JSON.stringify(result)}\n`);
		process.exitCode = EXIT.unavailable;
		return;
	}
	let folderId;
	try {
		folderId = folderIdNamed(session, FOLDER_NAME, ADMIN_TIMEOUT_MS);
	} catch {}
	for (const spec of specs) {
		const { slug, item } = parseSpec(spec);
		const matches = items.filter((entry) => entry.name === item);
		const chosen = matches.length > 0 ? pickItem(matches, folderId) : undefined;
		result.items[slug] = {
			item,
			exists: matches.length > 0,
			ambiguous: matches.length > 1,
			...(chosen ? { id: chosen.id } : {}),
		};
	}
	process.stdout.write(`${JSON.stringify(result)}\n`);
}

function main(argv) {
	const [first, ...rest] = argv;

	if (first === "--status") return printStatus();
	if (first === "--manifest") return printManifest(rest);
	if (first === "--forget") {
		forget(rest);
		return;
	}
	if (first === "--unlock") {
		const helper = unlockHelper();
		if (!helper) throw new Fail("no unlock helper found; set PI_BW_UNLOCK or run `bw unlock` yourself", EXIT.unavailable);
		const result = spawnSync(helper, [], { encoding: "utf8", stdio: ["inherit", "pipe", "inherit"], env: process.env });
		if (result.status !== 0) throw new Fail(`unlock helper failed (exit ${result.status})`, EXIT.locked);
		process.stdout.write(`${JSON.stringify(readStatus((result.stdout || "").trim() || undefined), null, 2)}\n`);
		return;
	}
	if (first === "--check") {
		const spec = parseSpec(rest[0] ?? "");
		// Says which store answers, never the value: this is the per-provider probe
		// for a migration, so "which store" is the entire answer.
		const keychain = fetchKeychain(spec.slug);
		if (keychain) {
			process.stdout.write(
				`${JSON.stringify({ slug: spec.slug, source: "keychain", service: keychainService(spec.slug) })}\n`,
			);
			return;
		}
		fetchKeyWithAnySession(spec.item);
		process.stdout.write(`${JSON.stringify({ slug: spec.slug, source: "vault", item: spec.item })}\n`);
		return;
	}
	if (!first || first.startsWith("--")) {
		throw new Fail(
			"usage: bw-key.mjs <slug> | --check <slug> | --manifest <slug[=Item]>... | --status | --unlock | --forget [<slug>...]\n" +
				"       a bare <slug> reads the login keychain first and falls back to the vault",
			EXIT.usage,
		);
	}

	const spec = parseSpec(first);
	// The keychain first, because this is pi's apiKey path and it runs once per
	// provider inside startup. A hit returns before the cache is consulted, so a
	// migrated key is never copied into the plaintext cache on disk.
	const fromKeychain = fetchKeychain(spec.slug);
	if (fromKeychain) {
		process.stdout.write(`${fromKeychain}\n`);
		return;
	}
	const cached = readCache(spec.slug);
	if (cached) {
		process.stdout.write(`${cached.key}\n`);
		return;
	}
	const { key, session } = fetchKeyWithAnySession(spec.item);
	writeCache(spec.slug, { key, item: spec.item, fetchedAt: Date.now(), sessionFrom: session });
	process.stdout.write(`${key}\n`);
}

try {
	main(process.argv.slice(2));
} catch (error) {
	if (error instanceof Fail) {
		// A bare slug failure is the request path; leaving a breadcrumb is the only
		// way the reason survives, because pi discards stderr of apiKey commands.
		const slug = (process.argv[2] || "").split("=")[0];
		if (slug && !slug.startsWith("--")) noteError(slug, error.message);
		process.stderr.write(`bw-key: ${error.message}\n`);
		process.exit(error.code);
	}
	process.stderr.write(`bw-key: ${error?.stack || error}\n`);
	process.exit(1);
}
