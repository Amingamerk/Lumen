import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { svelteTesting } from '@testing-library/svelte/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [tailwindcss(), svelte(), svelteTesting()],
  clearScreen: false,
  server: {
    // Must match devUrl in src-tauri/tauri.conf.json.
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    // WebView2 is evergreen Chromium; no legacy transforms needed.
    target: 'chrome120',
    // Fonts must stay files: the CSP only allows `font-src 'self'`, not data: URIs.
    assetsInlineLimit: 0,
  },
  test: {
    environment: 'jsdom',
    // Only the app's tests; .claude/hooks has its own node:test suite.
    include: ['src/**/*.test.ts'],
  },
});
