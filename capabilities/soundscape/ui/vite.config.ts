import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

// No proxy table here, unlike the spine dashboard: this surface talks to exactly one
// capability — its own — so the dev server only needs that one target. The port comes
// from the environment because service.toml is the single place it is declared.
const port = Number(process.env.SJEL_PORT ?? 8088);

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    proxy: {
      "/api/soundscape": { target: `http://127.0.0.1:${port}`, changeOrigin: true },
    },
  },
});
