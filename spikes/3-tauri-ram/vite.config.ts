import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  clearScreen: false,
  // Must match devUrl in src-tauri/tauri.conf.json.
  server: { port: 1420, strictPort: true },
});
