import { readFileSync } from "node:fs";
import type { IncomingHttpHeaders } from "node:http";
import { isAbsolute, join, resolve } from "node:path";

type RuntimeEnv = Partial<
  Pick<NodeJS.ProcessEnv, "SJEL_COMMS_CONFIG" | "SJEL_PERSONAL_ROOT" | "HOME">
>;

export interface CommsProxyCredential {
  authorization: string | null;
  reason: "configured" | "config-missing" | "secret-unconfigured" | "secret-unreadable";
}

interface ProxyRequestHeaders {
  setHeader(name: string, value: string): void;
}

interface ProxyRequest {
  method?: string;
}

interface ProxyEvents {
  on(
    event: "proxyReq",
    listener: (proxyRequest: ProxyRequestHeaders, request: ProxyRequest) => void,
  ): void;
}

function expandTilde(path: string, home: string | undefined): string {
  return path.startsWith("~/") && home ? join(home, path.slice(2)) : path;
}

function fromAxonRoot(path: string, axonRoot: string): string {
  return isAbsolute(path) ? path : resolve(axonRoot, path);
}

function configPath(axonRoot: string, env: RuntimeEnv): string {
  if (env.SJEL_COMMS_CONFIG) {
    return fromAxonRoot(expandTilde(env.SJEL_COMMS_CONFIG, env.HOME), axonRoot);
  }
  if (env.SJEL_PERSONAL_ROOT) {
    return join(expandTilde(env.SJEL_PERSONAL_ROOT, env.HOME), "config", "comms.json");
  }
  return join(axonRoot, "capabilities", "comms", "comms.config.json");
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

/**
 * Resolve the same Comms config and secret-file shapes as the Rust server. The
 * returned value stays in the Vite process; it is never imported by browser code.
 */
export function loadCommsProxyCredential(
  axonRoot: string,
  env: RuntimeEnv = process.env,
): CommsProxyCredential {
  let config: { api_secret_file?: unknown };
  try {
    config = JSON.parse(readFileSync(configPath(axonRoot, env), "utf8"));
  } catch {
    return { authorization: null, reason: "config-missing" };
  }

  if (typeof config.api_secret_file !== "string" || !config.api_secret_file.trim()) {
    return { authorization: null, reason: "secret-unconfigured" };
  }

  const secretPath = fromAxonRoot(
    expandTilde(config.api_secret_file.trim(), env.HOME),
    axonRoot,
  );
  try {
    const token = tokenFromBody(readFileSync(secretPath, "utf8"));
    return token
      ? { authorization: `Bearer ${token}`, reason: "configured" }
      : { authorization: null, reason: "secret-unreadable" };
  } catch {
    return { authorization: null, reason: "secret-unreadable" };
  }
}

export function isMutation(method: string | undefined): boolean {
  const normalized = method?.toUpperCase();
  return normalized !== "GET" && normalized !== "HEAD" && normalized !== "OPTIONS";
}

function firstHeader(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

/** Browser mutations must originate from the dashboard that received them. */
export function hasSameOrigin(headers: IncomingHttpHeaders): boolean {
  const origin = firstHeader(headers.origin);
  if (!origin) return true; // local non-browser clients remain usable
  const host = firstHeader(headers.host);
  if (!host) return false;
  try {
    const parsed = new URL(origin);
    return (
      (parsed.protocol === "http:" || parsed.protocol === "https:") &&
      parsed.host.toLowerCase() === host.toLowerCase()
    );
  } catch {
    return false;
  }
}

/**
 * Attach the private token to every proxied Comms request.
 *
 * Mutations only, until Comms moved onto the shared inbound gate in
 * `libs/axon-server`: that gate admits `/health` and `/ready` and asks every
 * other path for the token, reads included. Sending it on a read costs nothing —
 * the value stays in this Vite process and never reaches browser JavaScript,
 * exactly as on a write — while withholding it would 401 every Comms page.
 *
 * A preflight `OPTIONS` is harmless to sign and never reaches a handler; the
 * gate lets it through to the CORS layer regardless.
 */
export function installCommsProxyAuthorization(
  proxy: ProxyEvents,
  authorization: string,
): void {
  proxy.on("proxyReq", (proxyRequest) => {
    proxyRequest.setHeader("Authorization", authorization);
  });
}
