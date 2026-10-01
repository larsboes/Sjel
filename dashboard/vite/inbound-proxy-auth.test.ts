import { afterEach, describe, expect, it } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  installInboundProxyAuthorization,
  loadInboundProxyCredential,
} from "./inbound-proxy-auth";

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function overlay(): string {
  const root = mkdtempSync(join(tmpdir(), "sjel-auth-test-"));
  roots.push(root);
  mkdirSync(join(root, "config"));
  mkdirSync(join(root, "secrets"));
  return root;
}

describe("inbound proxy credential", () => {
  it("resolves the declared private token file without changing the declaration", () => {
    const root = overlay();
    writeFileSync(
      join(root, "config", "deployment.env"),
      "# ignored\nSJEL_INBOUND_TOKEN_FILE=" + join(root, "secrets", "token") + "\n",
    );
    writeFileSync(join(root, "secrets", "token"), "local-test-token\n");

    expect(loadInboundProxyCredential("/repo", { SJEL_OVERLAY_ROOT: root })).toEqual({
      authorization: "Bearer local-test-token",
      reason: "configured",
    });
  });

  it("reads the token from the auth.api_key field when the file is JSON", () => {
    const root = overlay();
    const keyPath = join(root, "secrets", "provider.json");
    writeFileSync(join(root, "config", "deployment.env"), `SJEL_INBOUND_TOKEN_FILE=${keyPath}\n`);
    writeFileSync(keyPath, JSON.stringify({ auth: { api_key: "local-test-token" } }));

    expect(loadInboundProxyCredential("/repo", { SJEL_OVERLAY_ROOT: root }).authorization).toBe(
      "Bearer local-test-token",
    );
  });

  it("fails closed when the deployment does not declare a token", () => {
    const root = overlay();
    writeFileSync(join(root, "config", "deployment.env"), "SJEL_HOME_TIMEZONE=Europe/Berlin\n");
    expect(loadInboundProxyCredential("/repo", { SJEL_OVERLAY_ROOT: root })).toEqual({
      authorization: null,
      reason: "declaration-missing",
    });
  });

  it("injects the shared token server-side but preserves agent credentials", () => {
    let handler:
      | ((
          request: { setHeader(name: string, value: string): void },
          incoming: { headers?: Record<string, string | string[] | undefined> },
        ) => void)
      | undefined;
    const proxy = {
      on(_event: "proxyReq", callback: typeof handler) {
        handler = callback;
      },
    };
    installInboundProxyAuthorization(proxy, "Bearer local-test-token");

    const browserHeaders = new Map<string, string>();
    handler?.({ setHeader: (name, value) => browserHeaders.set(name, value) }, { headers: {} });
    expect(browserHeaders.get("Authorization")).toBe("Bearer local-test-token");

    const agentHeaders = new Map<string, string>();
    handler?.(
      { setHeader: (name, value) => agentHeaders.set(name, value) },
      { headers: { authorization: "Bearer agent-token" } },
    );
    expect(agentHeaders.has("Authorization")).toBe(false);
  });
});
