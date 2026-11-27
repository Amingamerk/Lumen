// The palette follows the logo's gradient and stays readable (WCAG AA).
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import tauriConfig from '../src-tauri/tauri.conf.json';
import indexHtml from '../index.html?raw';
import logo from './lib/assets/lumen-logo.svg?raw';

// Vitest empties CSS imports, even `?raw` ones, so read the file.
const css = readFileSync('src/app.css', 'utf8');

function token(name: string): string {
  const value = css.match(new RegExp(`--color-${name}:\\s*(#[0-9a-f]{6});`))?.[1];
  if (!value) throw new Error(`no hex token --color-${name}`);
  return value;
}

/** Relative luminance, as WCAG 2 defines it. */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((at) => {
    const channel = parseInt(hex.slice(at, at + 2), 16) / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [low, high] = [luminance(token(a)), luminance(token(b))].sort((x, y) => x - y);
  return (high + 0.05) / (low + 0.05);
}

const surfaces = ['night', 'deck', 'stage', 'raise', 'lit'];
const pairs = (colors: string[], grounds: string[]) =>
  colors.flatMap((color) => grounds.map((ground) => [color, ground] as const));

describe('palette', () => {
  it.each(['iris', 'lume', 'ember'])('takes --color-%s from the logo gradient', (name) => {
    expect(logo).toContain(`stop-color="${token(name)}"`);
  });

  it.each(
    pairs(
      ['ink', 'ink-soft', 'ink-faint', 'lume-hi', 'youtube', 'soundcloud', 'spotify'],
      surfaces,
    ),
  )('%s reads as text on %s', (color, ground) => {
    expect(contrast(color, ground)).toBeGreaterThanOrEqual(4.5);
  });

  // Active icons, focus rings and the alert icon are graphics, not text.
  it.each(pairs(['lume', 'lume-hi', 'ember'], surfaces))(
    '%s stands out as a graphic on %s',
    (color, ground) => {
      expect(contrast(color, ground)).toBeGreaterThanOrEqual(3);
    },
  );

  it.each(['lume', 'lume-hi'])('night reads as text on %s', (ground) => {
    expect(contrast('night', ground)).toBeGreaterThanOrEqual(4.5);
  });

  it('paints the window the same night before the app loads', () => {
    expect(tauriConfig.app.windows[0].backgroundColor).toBe(token('night'));
    expect(indexHtml).toContain(`name="theme-color" content="${token('night')}"`);
  });
});
