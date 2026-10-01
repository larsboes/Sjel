import type { ProxyOptions } from "vite";

import { loadInboundCredential, type InboundCredential } from "../../tools/lib/inbound-auth.ts";

// The token itself is read in tools/lib/inbound-auth.ts, shared with the tools that call a
// capability. This file keeps only what is Vite's: attaching it to the proxy hop.
export type InboundProxyCredential = InboundCredential;
export const loadInboundProxyCredential = loadInboundCredential;

interface ProxyRequest {
  setHeader(name: string, value: string): void;
}

interface ProxyServer {
  on(
    event: "proxyReq",
    listener: (
      request: ProxyRequest,
      incoming: { headers?: Record<string, string | string[] | undefined> },
    ) => void,
  ): void;
}

/** Attach the credential to the server-to-server hop; the browser never sees it. */
export function installInboundProxyAuthorization(
  proxy: ProxyServer,
  authorization: string,
): void {
  proxy.on("proxyReq", (request, incoming) => {
    // Preserve a caller's agent token so the capability runs its read-only pseudonymization
    // branch. Otherwise inject the browser's deployment credential server-side.
    if (!incoming.headers?.authorization) request.setHeader("Authorization", authorization);
  });
}

export function authorizedProxy(
  authorization: string | null,
): Pick<ProxyOptions, "configure"> | undefined {
  if (!authorization) return undefined;
  return {
    configure: (proxy) => installInboundProxyAuthorization(proxy, authorization),
  };
}
