import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'node:path';

export default defineConfig(({ mode }) => ({
  root: import.meta.dirname,
  base: './',
  plugins: [svelte()],
  build: {
    target: 'safari16.4',
    outDir: mode === 'server-test' ? 'dist/server' : 'dist/client',
    emptyOutDir: false,
    ...(mode === 'server-test' ? { ssr: resolve(import.meta.dirname, 'render-entry.ts') } : {}),
  },
  ssr: { noExternal: ['@json-render/svelte'] },
}));
