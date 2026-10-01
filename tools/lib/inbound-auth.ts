// tools/lib/inbound-auth.ts — the deployment's inbound token, read for a loopback caller.
//
// Every serve_local capability refuses a protected route without a credential (ISA ISC-45,
// libs/sjel-server/src/lib.rs). A TypeScript caller on this machine reads the token here: the
// dashboard's Vite proxy, soundscape's dev proxy, and scheduled jobs such as
// tools/sparpreis-watch.ts. It lived in dashboard/vite/ until a tool needed it, and a tool
// importing from the dashboard is a dependency pointing the wrong way.
//
// The declaration is `SJEL_INBOUND_TOKEN_FILE` in <overlay>/config/deployment.env, and the
// token is that file's contents. libs/sjel-server's `deployment_token` reads the same pair.

import { readFileSync } from "node:fs";
import { isAbsolute, join, resolve } from "node:path";
import { execFileSync } from "node:child_process";

export type RuntimeEnv = Partial<
  Pick<NodeJS.ProcessEnv, "SJEL_OVERLAY_ROOT" | "SJEL_PERSONAL_ROOT" | "HOME">
>;

export interface InboundCredential {
  authorization: string | null;
  reason: "configured" | "declaration-missing" | "secret-unreadable";
}

function expandTilde(path: string, home: string | undefined): string {
  return path.startsWith("~/") && home ? join(home, path.slice(2)) : path;
}

function overlayRoot(repoRoot: string, env: RuntimeEnv): string {
  const explicit = env.SJEL_OVERLAY_ROOT ?? env.SJEL_PERSONAL_ROOT;
  if (explicit?.trim()) return resolve(expandTilde(explicit.trim(), env.HOME));
  const script = 'source "$1/tools/lib/paths.sh"; printf "%s" "$SJEL_OVERLAY_ROOT"';
  return execFileSync("bash", ["-c", script, "sjel-paths", repoRoot], { encoding: "utf8" }).trim();
}

function deploymentValue(text: string, name: string): string | null {
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const split = trimmed.indexOf("=");
    if (split < 0 || trimmed.slice(0, split).trim() !== name) continue;
    const value = trimmed.slice(split + 1).trim();
    return value.replace(/^"(.*)"$/, "$1").replace(/^'(.*)'$/, "$1") || null;
  }
  return null;
}

function tokenFromBody(body: string): string | null {
  const trimmed = body.trim();
  if (!trimmed) return null;
  try {
    const parsed = JSON.parse(trimmed) as { auth?: { api_key?: unknown } };
    const key = parsed?.auth?.api_key;
    return typeof key === "string" && key.trim() ? key.trim() : null;
  } catch {
    return trimmed;
  }
}

/** Read the deployment token. Server-side only: no browser code may import this module. */
export function loadInboundCredential(
  repoRoot: string,
  env: RuntimeEnv = process.env,
): InboundCredential {
  try {
    const root = overlayRoot(repoRoot, env);
    const declaration = readFileSync(join(root, "config", "deployment.env"), "utf8");
    const reference = deploymentValue(declaration, "SJEL_INBOUND_TOKEN_FILE");
    if (!reference) return { authorization: null, reason: "declaration-missing" };
    const expanded = expandTilde(reference, env.HOME);
    const secretPath = isAbsolute(expanded) ? expanded : resolve(expanded);
    const token = tokenFromBody(readFileSync(secretPath, "utf8"));
    return token
      ? { authorization: `Bearer ${token}`, reason: "configured" }
      : { authorization: null, reason: "secret-unreadable" };
  } catch {
    return { authorization: null, reason: "secret-unreadable" };
  }
}


/** Add the credential to a request, and refuse unless the target is this machine's loopback.
 *  The token must never reach a configured remote endpoint or a model provider. */
export function authorizedLoopbackRequest(
  url: string,
  init: RequestInit,
  authorization: string,
): { target: URL; init: RequestInit } {
  const target = new URL(url);
  if (!["127.0.0.1", "localhost", "[::1]"].includes(target.hostname)) {
    throw new Error("capability request target is not loopback");
  }
  const headers = new Headers(init.headers);
  headers.set("authorization", authorization);
  return { target, init: { ...init, headers } };
}
