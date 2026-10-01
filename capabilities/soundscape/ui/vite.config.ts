import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

import { loadInboundCredential } from "../../../tools/lib/inbound-auth";

// No proxy table here, unlike the spine dashboard: this surface talks to exactly one
// capability — its own — so the dev server only needs that one target. The port comes
// from the environment because service.toml is the single place it is declared.
const port = Number(process.env.SJEL_PORT ?? 8088);

// soundscape refuses a protected route without the deployment token (ISA ISC-45), so the
// dev server adds it to the proxy hop. The browser never sees it.
const repoRoot = new URL("../../../", import.meta.url).pathname;
const authorization = loadInboundCredential(repoRoot).authorization;

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    proxy: {
      "/api/soundscape": {
        target: `http://127.0.0.1:${port}`,
        changeOrigin: true,
        ...(authorization && {
          configure: (proxy) =>
            proxy.on("proxyReq", (request) => request.setHeader("Authorization", authorization)),
        }),
      },
    },
  },
});
