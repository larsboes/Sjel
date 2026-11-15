import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
export default defineConfig({ base: '/catalog/', plugins: [svelte()], build: { target: 'safari16.4' } });
