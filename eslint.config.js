import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import { defineConfig, globalIgnores } from 'eslint/config';
import globals from 'globals';
import ts from 'typescript-eslint';
import svelteConfig from './svelte.config.js';

export default defineConfig(
  // Spikes are throwaway code with their own setup; .claude has its own tests.
  globalIgnores(['dist/', 'src-tauri/', 'spikes/', '.claude/']),
  js.configs.recommended,
  ts.configs.strict,
  svelte.configs.recommended,
  { languageOptions: { globals: globals.browser } },
  {
    files: ['**/*.svelte', '**/*.svelte.ts'],
    languageOptions: {
      parserOptions: { parser: ts.parser, svelteConfig, extraFileExtensions: ['.svelte'] },
    },
  },
  {
    files: ['src/**/*.{ts,svelte}'],
    rules: {
      // Rust pushes state changes; the UI never polls.
      'no-restricted-globals': ['error', 'setInterval', 'fetch', 'XMLHttpRequest', 'WebSocket'],
    },
  },
);
