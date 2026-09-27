import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/**
 * adapter-static with an SPA fallback, not prerendering: every route here reads live
 * state from capabilities on this machine, so there is nothing true at build time to
 * render. The build is a static bundle any server can hand out, which is what makes
 * the eventual home-server deployment a file copy rather than a second architecture.
 *
 * @type {import("@sveltejs/kit").Config}
 */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({ pages: "dist", assets: "dist", fallback: "index.html", strict: false }),
    // Empty on a real machine, where the shell is served from the root of its own port.
    // The published demo (#168) is served from a subdirectory of a GitHub Pages site, and a
    // static SPA cannot discover that at runtime — every asset URL and every router link is
    // baked in at build time. tools/demo-site sets it; nothing else does.
    paths: { base: process.env.SJEL_DEMO_BASE ?? "" },
  },
};

export default config;
