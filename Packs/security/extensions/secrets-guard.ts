/**
 * Secrets Guard — pi agent extension
 *
 * Prevents the agent from reading secrets files (.env, .secrets, *credentials*),
 * Bitwarden items, and other sensitive material. Blocks at the tool-call boundary
 * before any data reaches the LLM. A provider the overlay lists in
 * `config/secrets-guard.toml` (`raw_secret_providers = [...]`) may read raw secrets,
 * and only in ephemeral (`--no-session`) sessions, where tool results are not
 * persisted to JSONL. Sjel itself trusts no provider: without that file the list is empty.
 *
 * == What is blocked ==
 *
 *   read  — any path matching *.env, *.secrets, *credentials*, *.pem, *.key,
 *           ~/.netrc, ~/.git-credentials, git credential store, kubeconfig,
 *           ~/.ssh/id_* private keys
 *
 *   ls/grep/find — listing/searching a secret path, or a pattern/glob that
 *         targets those files (e.g. `find . -name '*.env'`)
 *
 *   bash  — cat/grep/head/tail/less/more/nl/wc/type on those file patterns
 *         — bw get, bw list, bw sync (reading secrets from Bitwarden)
 *         — echo $SECRET_VAR (echoing env vars that match known secret names)
 *         — env, env | …, env > f (dumping the whole environment; the word
 *           "env" in `process.env.X`, `env_file:` or `environment:` is not that)
 *
 *   edit/write — any mutation targeting a path matching *.env, *.secrets, etc.
 *
 * == What is allowed ==
 *
 *   bash  — source .env && …  (sources secrets into env, doesn't print them)
 *         — bw unlock          (unlocks vault, session token not a secret value)
 *         — bw encode, bw generate
 *         — export VAR=…
 *
 * == Custom tool: vault_exec ==
 *
 *   A registered tool that runs a command with an env file sourced, captures
 *   stdout/stderr, and strips env-file values from the output before the LLM
 *   sees it. Use when you need to call an API that needs a credential:
 *
 *     vault_exec({cmd: "curl -s https://api.example.com", env_file: ".env"})
 *
 *   The secret values are never part of the tool-call input the LLM constructs
 *   and never appear in the output — the extension resolves them directly.
 */

import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { chmodSync, readFileSync, existsSync, realpathSync } from "node:fs";
import { execSync } from "node:child_process";
import { basename, dirname, resolve } from "node:path";

// ---- patterns -----------------------------------------------------------

/** File path patterns that contain secrets — blocked from read/edit. */
const SECRET_FILE_PATTERNS: RegExp[] = [
  /\.env$/,
  /\.env\.[a-zA-Z0-9_.-]+$/,   // .env.local, .env.production, .env.development
  /\.secrets?$/,
  /\.secrets?\.[a-zA-Z0-9_.-]+$/,
  /credentials?\./i,
  /\.pem$/,
  /\.key$/,
  /\.p12$/,
  /\.pfx$/,
  /token\./i,
  /secret\./i,
  // credential stores & private keys with no tell-tale extension
  /(^|\/)\.netrc$/,
  /(^|\/)\.git-credentials$/,
  /(^|\/)config\/git\/credentials$/,
  /kubeconfig/i,
  /(^|\/)\.ssh\/id_/,          // id_rsa, id_ed25519, … (no extension)
];

/** Bash commands or patterns that leak secrets. */
const SECRET_BASH_PATTERNS: RegExp[] = [
  // cat/grep/head/tail etc. on secret files
  /\b(cat|bat|less|more|head|tail|nl|tac|rev)\s+.*\.env/,
  /\b(cat|bat|less|more|head|tail|nl|tac|rev)\s+.*\.secrets?/,
  /\b(grep|rg|ag|ack|find|sed)\s+.*\.env/,
  /\b(grep|rg|ag|ack|find|sed)\s+.*\.secrets?/,
  /\b(wc|sort|uniq)\s+.*\.env/,
  /\b(wc|sort|uniq)\s+.*\.secrets?/,
  // Bitwarden read operations
  /\bbw\s+get\b/,
  /\bbw\s+list\b/,
  /\bbw\s+sync\b/,
  /\bbw\s+export\b/,
  // Printing env vars that likely hold secrets
  /\becho\s+\$[A-Z_]*(?:TOKEN|SECRET|KEY|PASSWORD|PASS|CREDENTIAL|API_KEY|SECRET_)\w*/,
  /\bprintenv\b/,
  // A bare `env` dump: `env` at a command position, optionally with flags, then a
  // pipe, a redirect, a separator or the end of the command. Anchored this way,
  // the word "env" inside `process.env.X = …`, `env_file:` or `environment:` is
  // not a dump, while `env`, `env | grep`, `env > f`, `x; env` and `$(env)` are.
  // The previous rule here required a trailing newline, so it missed a plain
  // `env` (the common case) and blocked any multi-line command mentioning env.
  /(^|[;&|()`]\s*|\bsudo\s+)env(\s+-{1,2}[\w-]+)*\s*($|[|>;&)`])/m,
  // OpenSSL reading private keys
  /\bopenssl\s+(?:pkey|rsa|ec|dsa)\s+.*-in\s+.*\.(?:pem|key)/,
];

/**
 * Providers the overlay trusts with raw secrets. Which providers qualify is a machine
 * decision, so the list lives in the overlay's `config/secrets-guard.toml` as one line:
 * `raw_secret_providers = ["name", ...]`. A missing or unreadable file means none.
 */
function loadRawSecretProviders(): Set<string> {
  const overlay = process.env.SJEL_OVERLAY_ROOT ?? process.env.SJEL_PERSONAL_ROOT;
  if (!overlay) return new Set();
  const path = resolve(overlay, "config", "secrets-guard.toml");
  if (!existsSync(path)) return new Set();
  try {
    const line = readFileSync(path, "utf-8").match(/^raw_secret_providers\s*=\s*\[([^\]]*)\]/m);
    if (!line) return new Set();
    return new Set([...line[1].matchAll(/"([^"]+)"/g)].map((m) => m[1].toLowerCase()));
  } catch {
    return new Set();
  }
}

const RAW_SECRET_PROVIDERS = loadRawSecretProviders();

/** Env-var name patterns that hold secrets — stripped from tool output. */

const SECRET_VAR_PATTERNS: RegExp[] = [
  /TOKEN/i,
  /SECRET/i,
  /PASSWORD/i,
  /PASS(?:_|$)/i,
  /API_KEY/i,
  /APIKEY/i,
  /CREDENTIAL/i,
  /AUTH/i,
  /_KEY$/i,
  /_ID$/i,
];

// ---- helpers -----------------------------------------------------------

/** Raw secret output is allowed only for approved providers without session persistence. */
function canReceiveRawSecrets(ctx: ExtensionContext): boolean {
  const provider = ctx.model?.provider.toLowerCase();
  return Boolean(
    provider &&
    RAW_SECRET_PROVIDERS.has(provider) &&
    !ctx.sessionManager.getSessionFile()
  );
}

/** Check if a path matches any secret-file pattern. */
function isSecretPath(path: string): boolean {
  const normalized = path.replace(/\\/g, "/");
  return SECRET_FILE_PATTERNS.some((p) => p.test(normalized));
}

/** Resolve symlinks, including symlinked parents of a path that does not exist yet. */
function canonicalPath(rawPath: string, cwd: string): string {
  const lexical = resolve(cwd, rawPath);
  let existing = lexical;
  const missingSegments: string[] = [];

  while (!existsSync(existing)) {
    const parent = dirname(existing);
    if (parent === existing) return lexical;
    missingSegments.unshift(basename(existing));
    existing = parent;
  }

  return resolve(realpathSync(existing), ...missingSegments);
}

function isSecretToolPath(rawPath: string, cwd: string): boolean {
  return isSecretPath(rawPath) || isSecretPath(canonicalPath(rawPath, cwd));
}

/** Build a readable list of matched patterns for the block reason. */
function matchReason(path: string): string {
  const matched = SECRET_FILE_PATTERNS
    .filter((p) => p.test(path))
    .map((p) => p.source);
  return matched.length > 0 ? `(${matched.join(", ")})` : "(pattern match)";
}

/**
 * Tokens that look like a secret file but name a variable namespace in code:
 * `process.env.X`, `import.meta.env`, `os.environ`, `$env:PATH`. They are removed
 * before the secret-file patterns run, so searching code for `process.env` is not
 * mistaken for reading a dotenv file. A genuine `.env` path in the same command
 * survives the removal and still blocks it.
 */
const ENV_NAMESPACE = /\b(?:process|import\.meta|Deno|Bun)\.env\b|\bos\.environ\b|\$env:(?=[A-Za-z_])/g;

/**
 * Every secret-looking path a command mentions.
 *
 * Extracted generously on purpose. The rule that uses this is an inversion: rather than
 * enumerating readers (`cat`, `head`, `awk`, `python3 -c`, `base64`, `tar`, `git show` …) and
 * losing to whichever one nobody listed, ANY reference to a secret path is refused unless the
 * command is one of the allowlisted shapes below. Over-extraction costs a blocked command with a
 * reason the caller can act on; under-extraction costs the secret.
 *
 * Splitting on shell metacharacters is what makes the embedded cases land: in
 * `python3 -c "open('.env').read()"` and `git show HEAD:.env` the split still yields `.env`, which
 * isSecretPath matches on its own.
 */
function secretPathsIn(command: string): string[] {
  const found = new Set<string>();
  for (const token of command.split(/[\s'"`|;&()<>=,:\[\]{}$\\]+/)) {
    if (token.length < 3) continue;
    if (isSecretPath(token)) found.add(token);
  }
  return [...found];
}

/**
 * Whether a command only NAMES a secret path rather than reading it.
 *
 * `printf 'env_file: .env\n' > compose.yml` writes a compose service that references the file; it
 * never opens it. Refusing that would obstruct ordinary config work while protecting nothing, and
 * the existing extension test encodes it as an allowed case.
 *
 * The exemption is deliberately narrow, because quoting alone proves nothing — an inline script
 * reads a quoted path (`python3 -c "open('.env').read()"`). It needs both a quoted path AND a verb
 * that only ever writes text, and `grep -n TOKEN .env` still blocks because there the path is bare.
 */
function onlyMentionsSecretPath(command: string, probe: string): boolean {
  const firstWord = (command.trim().split(/[\s;|&(]+/)[0] ?? "").toLowerCase();
  if (firstWord !== "printf" && firstWord !== "echo") return false;
  return /['"][^'"]*\.(?:env|secrets?)(?:\.[A-Za-z0-9_.-]+)?[^'"]*['"]/.test(probe);
}

function isSecretReadCommand(command: string): { blocked: boolean; reason?: string } {
  // The command as it should be judged: namespaces that merely contain "env" are
  // blanked out first, so `rg process.env` is not a read of `.env`.
  const probe = command.replace(ENV_NAMESPACE, "ENV_NAMESPACE");

  // Allow `source` — it loads vars into env without printing them
  if (/^(\s*source\s+|\s*\.\s+)[^\n&|;]*\.(env|secrets?)\b/.test(command)) {
    // But still check if the same command also tries to cat/grep the file
    for (const pattern of SECRET_BASH_PATTERNS) {
      if (pattern.test(probe)) {
        return { blocked: true, reason: `matches secret-read pattern: ${pattern.source}` };
      }
    }
    return { blocked: false };
  }

  // Allow `export` — setting vars is not reading
  if (/^\s*export\s+[A-Za-z_][A-Za-z0-9_]*=/.test(command)) {
    return { blocked: false };
  }

  // Allow `bw unlock` (sets session, doesn't read secrets)
  if (/^\s*bw\s+unlock\b/.test(command)) {
    return { blocked: false };
  }

  // Allow `bw encode` / `bw generate`
  if (/^\s*bw\s+(encode|generate)\b/.test(command)) {
    return { blocked: false };
  }

  // Check against known secret-leaking patterns
  for (const pattern of SECRET_BASH_PATTERNS) {
    if (pattern.test(probe)) {
      return { blocked: true, reason: `matches secret-read pattern: ${pattern.source}` };
    }
  }

  // The inversion. The blocklist above only knows the readers somebody thought of, and shell
  // indirection is unbounded — a pipe to base64, an inline script, a copy to a temporary file all
  // read the file without matching any of it. So anything that names a secret path and is not one
  // of the allowlisted shapes above is refused, whether or not a pattern recognises the reader.
  // Run against `probe`, not `command`: `process.env` ends in `.env` and would otherwise be refused
  // as a secret file, which is precisely the false positive ENV_NAMESPACE exists to prevent.
  const referenced = secretPathsIn(probe);
  if (referenced.length > 0 && !onlyMentionsSecretPath(command, probe)) {
    return {
      blocked: true,
      reason:
        `references a secret file (${referenced.join(", ")}). Known readers are blocked by name, but ` +
        `the reader here was not recognised — so it is refused rather than allowed. ` +
        `If this is a false positive, rephrase so the path is not named: the file's contents are ` +
        `available through vault_exec({cmd, env_file})`,
    };
  }

  return { blocked: false };
}

/** Strip known secret values from a string (for tool-result sanitization). */
function sanitizeOutput(text: string): string {
  if (!text) return text;

  let result = text;

  // Strip lines that look like secret env assignments
  result = result.replace(
    /^(export\s+)?([A-Za-z_][A-Za-z0-9_]*)=['"]?[^\s'"]{8,}['"]?$/gm,
    (match, _export, key) => {
      if (SECRET_VAR_PATTERNS.some((p) => p.test(key))) {
        return `${_export || ""}${key}=****`;
      }
      return match;
    },
  );

  // Strip base64-encoded OAuth tokens and similar long random strings
  // Typical refresh tokens are long (300+ chars)
  result = result.replace(/[A-Za-z0-9_-]{300,}(?:[=]{1,2})?/g, "****");

  return result;
}

/** Load an env file and return the parsed key-value pairs (values never returned). */
function loadEnvFile(envPath: string): Record<string, string> {
  const resolved = envPath.startsWith("/")
    ? envPath
    : resolve(process.cwd(), envPath);

  if (!existsSync(resolved)) {
    throw new Error(`env file not found: ${resolved}`);
  }

  const body = readFileSync(resolved, "utf-8");
  const vars: Record<string, string> = {};

  for (const line of body.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eqIdx = trimmed.indexOf("=");
    if (eqIdx <= 0) continue;
    const key = trimmed.slice(0, eqIdx).trim();
    let value = trimmed.slice(eqIdx + 1).trim();
    // Strip surrounding quotes
    if ((value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'"))) {
      value = value.slice(1, -1);
    }
    vars[key] = value;
  }

  return vars;
}

/** Run a command with an env file sourced, capture output, sanitize it. */
function runWithEnv(cmd: string, envFile?: string): string {
  const env = { ...process.env };

  if (envFile) {
    const loaded = loadEnvFile(envFile);
    for (const [key, val] of Object.entries(loaded)) {
      env[key] = val;
    }
  }

  const result = execSync(cmd, {
    env,
    encoding: "utf-8",
    maxBuffer: 10 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 30_000,
  });

  return sanitizeOutput(result);
}

// ---- extension ---------------------------------------------------------

export default function (pi: ExtensionAPI) {
  const hardenSessionFile = (ctx: ExtensionContext): void => {
    const sessionFile = ctx.sessionManager.getSessionFile();
    if (!sessionFile || process.platform === "win32" || !existsSync(sessionFile)) return;

    try {
      chmodSync(sessionFile, 0o600);
    } catch (error) {
      ctx.ui?.notify?.(
        `[secrets-guard] could not set session file permissions to 600: ${String(error)}`,
        "warning",
      );
    }
  };

  pi.on("session_start", async (_event, ctx) => hardenSessionFile(ctx));
  pi.on("agent_end", async (_event, ctx) => hardenSessionFile(ctx));

  // ── Block reads of secret files ───────────────────────────────────────

  pi.on("tool_call", async (event, ctx) => {
    const rawSecretsAllowed = canReceiveRawSecrets(ctx);

    if (event.toolName === "read") {
      const path = event.input.path as string;
      if (isSecretToolPath(path, ctx.cwd) && !rawSecretsAllowed) {
        ctx.ui?.notify?.(
          `[secrets-guard] blocked read of ${path} — use vault_exec for commands needing credentials`,
          "warning",
        );
        return {
          block: true,
          reason: `secrets-guard: reading ${path} would expose secrets (${matchReason(path)}). Run commands with vault_exec({cmd: "...", env_file: "${path}"}) instead — it sources the file without exposing values.`,
        };
      }
    }

    if (event.toolName === "edit" || event.toolName === "write") {
      const path = event.input.path as string;
      if (isSecretToolPath(path, ctx.cwd)) {
        return {
          block: true,
          reason: `secrets-guard: ${event.toolName} on ${path} is blocked — secret files cannot be mutated through model tools.`,
        };
      }
    }

    if (
      !rawSecretsAllowed &&
      (event.toolName === "ls" || event.toolName === "grep" || event.toolName === "find")
    ) {
      const input = event.input as { path?: string; pattern?: string; glob?: string };
      const p = input.path ?? "";
      if (p && isSecretToolPath(p, ctx.cwd)) {
        return {
          block: true,
          reason: `secrets-guard: ${event.toolName} on ${p} could expose secrets ${matchReason(p)}. Use vault_keys to list env-file variable names without values.`,
        };
      }
      // A pattern/glob that targets secret files (e.g. find -name '*.env')
      const needle = `${input.pattern ?? ""} ${input.glob ?? ""}`;
      if (needle.trim() && SECRET_FILE_PATTERNS.some((rx) => rx.test(needle))) {
        return {
          block: true,
          reason: `secrets-guard: ${event.toolName} pattern/glob targets secret files (${needle.trim()}).`,
        };
      }
    }

    if (event.toolName === "bash" && !rawSecretsAllowed) {
      const command = event.input.command as string;
      const check = isSecretReadCommand(command);
      if (check.blocked) {
        ctx.ui?.notify?.(
          `[secrets-guard] blocked bash command that would read secrets`,
          "warning",
        );
        return {
          block: true,
          reason: `secrets-guard: blocked — ${check.reason}. Use vault_exec({cmd: "...", env_file: ".env"}) to run commands with env vars sourced without exposing values. Or source the env file with \`source .env && your_command\`.`,
        };
      }
    }
  });

  // ── Sanitize tool results so leaked secrets don't reach the LLM ──────

  pi.on("tool_result", async (event, ctx) => {
    if (canReceiveRawSecrets(ctx)) return;

    if (event.toolName === "bash" || event.toolName === "read") {
      const sanitized = event.content?.map((block) => {
        if (block.type === "text") {
          return { ...block, text: sanitizeOutput(block.text) };
        }
        return block;
      });

      if (sanitized) {
        return {
          content: sanitized,
          details: event.details,
          isError: event.isError,
        };
      }
    }
  });

  // ── Register vault_exec tool ──────────────────────────────────────────

  pi.registerTool({
    name: "vault_exec",
    label: "Vault Exec",
    description:
      "Run a shell command with a minimal environment. When env_file is supplied, keys is required and only those variables plus PATH and HOME are exposed. The extension reads the file directly, and strips exposed values from output. Use for API calls or commands that need credentials from .env / .secrets files.",

    parameters: Type.Object({
      cmd: Type.String({
        description: "The shell command to run (e.g., 'curl -s https://api.example.com/endpoint')",
      }),
      env_file: Type.Optional(
        Type.String({
          description: "Path to an env file to source before running the command (e.g., '.env' or 'config/secrets.env'). The file is read by the extension, never seen by the LLM.",
        }),
      ),
      keys: Type.Optional(
        Type.Array(Type.String(), {
          description:
            "Names of the environment variables to expose. Required with env_file. Only these variables plus PATH and HOME exist in the child environment.",
        }),
      ),
      timeout: Type.Optional(
        Type.Number({
          description: "Timeout in seconds (default 30)",
        }),
      ),
    }),

    async execute(toolCallId, params, signal, onUpdate, ctx) {
      // execSync cannot be interrupted, so a call that is already cancelled is
      // refused up front rather than pretending the signal is honoured mid-command.
      if (signal?.aborted) throw new Error("vault_exec: cancelled before the command started");

      /**
       * Every value this call put into the child environment.
       *
       * sanitizeOutput is SHAPE-based: it redacts lines that look like `KEY=value` with a
       * secret-looking key. That misses the value printed on its own, which is exactly what
       * `printenv API_TOKEN` produces — and this tool is the one place that knows the values, so it
       * can strip them by value as well as by shape. Held outside the try so the error path, which
       * is where a failing command dumps its stderr, gets the same treatment.
       */
      const exposedValues: string[] = [];
      const scrub = (text: string): string => {
        let out = sanitizeOutput(text);
        for (const value of exposedValues) {
          if (value.length >= 8) out = out.split(value).join("****");
        }
        return out;
      };

      try {
        const wanted = params.keys && params.keys.length > 0 ? [...new Set(params.keys)] : [];
        if (params.env_file && wanted.length === 0) {
          throw new Error("vault_exec: keys is required when env_file is supplied");
        }

        const env: NodeJS.ProcessEnv = {
          PATH: process.env.PATH,
          HOME: process.env.HOME,
        };
        const source = params.env_file ? loadEnvFile(params.env_file) : process.env;
        const present = wanted.filter((key) => source[key] !== undefined);
        const absent = wanted.filter((key) => source[key] === undefined);
        if (absent.length > 0) {
          const sourceName = params.env_file ?? "the Pi process environment";
          throw new Error(
            `vault_exec: ${absent.join(", ")} not present in ${sourceName}` +
              (present.length > 0 ? ` (found: ${present.join(", ")})` : ""),
          );
        }

        for (const key of present) {
          const value = source[key];
          if (value === undefined) continue;
          env[key] = value;
          exposedValues.push(value);
        }

        if (params.env_file) {
          ctx.ui?.notify?.(
            `[vault_exec] sourced ${params.env_file} (${present.length} scoped var${present.length === 1 ? "" : "s"})`,
            "info",
          );
        }

        const result = execSync(params.cmd, {
          env,
          encoding: "utf-8",
          maxBuffer: 10 * 1024 * 1024,
          stdio: ["ignore", "pipe", "pipe"],
          timeout: (params.timeout ?? 30) * 1000,
        });

        const sanitized = scrub(result);

        return {
          content: [{ type: "text", text: sanitized }],
          details: {
            exitCode: 0,
            cmd: params.cmd,
            env_file: params.env_file ?? null,
          },
        };
      } catch (error: unknown) {
        const err = error as Error & { stderr?: string; stdout?: string; status?: number };
        const stderr = err.stderr ? scrub(err.stderr) : "";
        const stdout = err.stdout ? scrub(err.stdout) : "";
        const message = scrub(err.message || String(error));

        const content: { type: "text"; text: string }[] = [{ type: "text", text: stdout || message }];
        if (stderr) content.push({ type: "text", text: `stderr: ${stderr}` });

        return {
          content,
          details: {
            exitCode: err.status ?? -1,
            cmd: params.cmd,
            env_file: params.env_file ?? null,
          },
          isError: true,
        };
      }
    },
  });

  // ── Register vault_read tool (safe read of env files — shows keys, not values) ──

  pi.registerTool({
    name: "vault_keys",
    label: "Vault Keys",
    description:
      "List the variable names (not values) from an env file. Use this to discover what secrets are available without exposing their values.",

    parameters: Type.Object({
      env_file: Type.String({
        description: "Path to env file (e.g., '.env', 'config/secrets.env')",
      }),
    }),

    async execute(toolCallId, params, signal, onUpdate, ctx) {
      try {
        const loaded = loadEnvFile(params.env_file);
        const keys = Object.keys(loaded).sort();

        return {
          content: [{
            type: "text",
            text: `Keys in ${params.env_file} (${keys.length} vars):\n${keys.map((k) => `  ${k}`).join("\n")}`,
          }],
          details: {
            count: keys.length,
            env_file: params.env_file,
          },
        };
      } catch (error) {
        return {
          content: [{ type: "text", text: `Error reading ${params.env_file}: ${error}` }],
          isError: true,
          details: {},
        };
      }
    },
  });
}
