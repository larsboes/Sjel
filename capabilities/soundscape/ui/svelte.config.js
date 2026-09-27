import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/**
 * Same shape as the spine dashboard's: a static bundle with an SPA fallback, not
 * prerendering. Nothing here is true at build time — the page reads the conductor's
 * state at runtime — and a static bundle is what lets the Rust binary serve this
 * over the capability's own HTTP surface instead of needing a second server.
 *
 * @type {import("@sveltejs/kit").Config}
 */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({ pages: "dist", assets: "dist", fallback: "index.html", strict: false }),

    // SvelteKit defaults this to Date.now(). It lands in _app/version.json, reaches
    // the entry chunks, and changes their content hashes — so two identical builds
    // produce different bytes under different filenames, and a served bundle stops
    // being comparable to the one CI built. Read from the environment instead, so a
    // deploy step can still stamp something meaningful (a commit sha) without the
    // default build being nondeterministic.
    version: { name: process.env.SJEL_BUILD_VERSION ?? "dev" },
  },
};

export default config;
