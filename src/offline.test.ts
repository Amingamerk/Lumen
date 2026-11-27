// The frontend never calls external services: not from code, markup, styles or the CSP.
import { describe, expect, it } from 'vitest';
import tauriConfig from '../src-tauri/tauri.conf.json';
import indexHtml from '../index.html?raw';

const sources = import.meta.glob<string>(['./**/*.{ts,svelte,css}', '!./**/*.test.ts'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

/** `www.w3.org` is the SVG XML namespace, which is a name and never fetched. */
const EXTERNAL_URL = /(https?:|wss?:|["'(]\/\/)(?!\/?\/?www\.w3\.org\/)/i;

describe('offline frontend', () => {
  it('reads the app sources', () => {
    expect(Object.keys(sources)).toContain('./App.svelte');
    expect(Object.keys(sources)).toContain('./app.css');
  });

  it.each(Object.entries({ ...sources, 'index.html': indexHtml }))(
    '%s links nothing external and has no media element',
    (_path, text) => {
      expect(text).not.toMatch(EXTERNAL_URL);
      expect(text).not.toMatch(/<(audio|video)\b|new Audio\(/i);
    },
  );

  const csp: Record<string, string> = tauriConfig.app.security.csp;

  it('falls back to same-origin only', () => {
    expect(csp['default-src']).toBe("'self'");
  });

  it.each(['script-src', 'font-src'])('%s allows only bundled files', (directive) => {
    expect(csp[directive]).toBe("'self'");
  });

  it('allows images from the bundle and inline data only', () => {
    expect(csp['img-src']).toBe("'self' data:");
  });

  it('allows connections to the Tauri IPC bridge only', () => {
    expect(csp['connect-src']).toBe('ipc: http://ipc.localhost');
  });

  it('blocks webview media, so audio can only come from Rust', () => {
    expect(csp['media-src']).toBe("'none'");
  });
});
