import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  hasSameOrigin,
  installCommsProxyAuthorization,
  isMutation,
  loadCommsProxyCredential,
} from "./comms-proxy-auth";

function fixture(): { root: string; overlay: string } {
  const root = mkdtempSync(join(tmpdir(), "axon-comms-proxy-"));
  const overlay = join(root, "overlay");
  mkdirSync(join(overlay, "config"), { recursive: true });
  mkdirSync(join(overlay, "secrets"), { recursive: true });
  return { root, overlay };
}

describe("Comms proxy credential resolution", () => {
  test("reads the overlay config and a raw token without exposing it to browser code", () => {
    const { root, overlay } = fixture();
    writeFileSync(join(overlay, "config", "comms.json"), JSON.stringify({
      api_secret_file: join(overlay, "secrets", "comms-api-token"),
    }));
    writeFileSync(join(overlay, "secrets", "comms-api-token"), "test-token\n");

    expect(loadCommsProxyCredential(root, { SJEL_PERSONAL_ROOT: overlay })).toEqual({
      authorization: "Bearer test-token",
      reason: "configured",
    });
  });

  test("supports the JSON auth.api_key shape used by oMLX settings", () => {
    const { root, overlay } = fixture();
    writeFileSync(join(overlay, "config", "comms.json"), JSON.stringify({
      api_secret_file: "secrets/settings.json",
    }));
    mkdirSync(join(root, "secrets"));
    writeFileSync(join(root, "secrets", "settings.json"), JSON.stringify({
      auth: { api_key: "json-token" },
    }));

    expect(loadCommsProxyCredential(root, { SJEL_PERSONAL_ROOT: overlay })).toEqual({
      authorization: "Bearer json-token",
      reason: "configured",
    });
  });

  test("fails closed for an absent field or unreadable file", () => {
    const { root, overlay } = fixture();
    const config = join(overlay, "config", "comms.json");
    writeFileSync(config, "{}");
    expect(loadCommsProxyCredential(root, { SJEL_PERSONAL_ROOT: overlay }).reason)
      .toBe("secret-unconfigured");

    writeFileSync(config, JSON.stringify({ api_secret_file: "/missing/token" }));
    expect(loadCommsProxyCredential(root, { SJEL_PERSONAL_ROOT: overlay }).reason)
      .toBe("secret-unreadable");
  });

  test("fails closed for malformed config and JSON without auth.api_key", () => {
    const { root, overlay } = fixture();
    const config = join(overlay, "config", "comms.json");
    writeFileSync(config, "not json");
    expect(loadCommsProxyCredential(root, { SJEL_PERSONAL_ROOT: overlay }).reason)
      .toBe("config-missing");

    const settings = join(overlay, "secrets", "settings.json");
    writeFileSync(config, JSON.stringify({ api_secret_file: settings }));
    writeFileSync(settings, JSON.stringify({ auth: {} }));
    expect(loadCommsProxyCredential(root, { SJEL_PERSONAL_ROOT: overlay }).reason)
      .toBe("secret-unreadable");
  });
});

describe("Comms proxy request boundary", () => {
  test("recognizes methods that change state", () => {
    expect(isMutation("POST")).toBe(true);
    expect(isMutation("PATCH")).toBe(true);
    expect(isMutation("GET")).toBe(false);
    expect(isMutation("OPTIONS")).toBe(false);
  });

  test("accepts same-origin and non-browser requests but rejects hostile origins", () => {
    expect(hasSameOrigin({ host: "127.0.0.1:47117", origin: "http://127.0.0.1:47117" }))
      .toBe(true);
    expect(hasSameOrigin({ host: "127.0.0.1:47117" })).toBe(true);
    expect(hasSameOrigin({ host: "127.0.0.1:47117", origin: "https://attacker.example" }))
      .toBe(false);
    expect(hasSameOrigin({ host: "127.0.0.1:47117", origin: "null" })).toBe(false);
  });

  // The shared credential serves the browser; an agent's own credential must
  // survive so Comms can apply its read-only pseudonymization middleware.
  test("injects the shared token but preserves an agent token", () => {
    let listener:
      | ((
          request: { setHeader(name: string, value: string): void },
          incoming: { method?: string; headers?: IncomingHttpHeaders },
        ) => void)
      | undefined;
    installCommsProxyAuthorization({
      on(_event, next) {
        listener = next;
      },
    }, "Bearer test-token");

    const browserHeaders = new Map<string, string>();
    listener?.(
      { setHeader: (name, value) => browserHeaders.set(name, value) },
      { method: "GET", headers: {} },
    );
    expect(browserHeaders.get("Authorization")).toBe("Bearer test-token");

    const agentHeaders = new Map<string, string>();
    listener?.(
      { setHeader: (name, value) => agentHeaders.set(name, value) },
      { method: "GET", headers: { authorization: "Bearer agent-token" } },
    );
    expect(agentHeaders.has("Authorization")).toBe(false);
  });
});
