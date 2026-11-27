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

type Rgb = readonly [number, number, number];

function hexToRgb(hex: string): Rgb {
  return [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16)) as unknown as Rgb;
}

/** Alpha-composite a translucent sRGB foreground over an opaque background. */
function compositeOver(fg: Rgb, alpha: number, bg: Rgb): Rgb {
  return [
    fg[0] * alpha + bg[0] * (1 - alpha),
    fg[1] * alpha + bg[1] * (1 - alpha),
    fg[2] * alpha + bg[2] * (1 - alpha),
  ];
}

/** Relative luminance, as WCAG 2 defines it. */
function luminanceRgb([rByte, gByte, bByte]: Rgb): number {
  const [r, g, b] = [rByte, gByte, bByte].map((value) => {
    const channel = value / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrastRgb(a: Rgb, b: Rgb): number {
  const [low, high] = [luminanceRgb(a), luminanceRgb(b)].sort((x, y) => x - y);
  return (high + 0.05) / (low + 0.05);
}

function contrast(a: string, b: string): number {
  return contrastRgb(hexToRgb(token(a)), hexToRgb(token(b)));
}

function ruleBody(selectorPattern: RegExp): string {
  const match = css.match(selectorPattern);
  if (!match) throw new Error(`no CSS rule matching ${selectorPattern}`);
  return match[1];
}

function colorMixStops(block: string): Array<{ name: string; alpha: number }> {
  const matches = [...block.matchAll(/color-mix\(in srgb,\s*var\(--color-([a-z-]+)\)\s*(\d+(?:\.\d+)?)%,\s*transparent\)/g)];
  if (matches.length === 0) throw new Error('expected translucent color-mix stops');
  return matches.map((m) => ({ name: m[1], alpha: Number(m[2]) / 100 }));
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

  it('uses the light accent for every focus ring and keeps contrast over translucent surfaces', () => {
    const focusRules = [
      /(?:^|\n)\s*:focus-visible\s*\{([^}]*)\}/,
      /\.search-field:has\(input:focus-visible\)\s*\{([^}]*)\}/,
      /\.chip:has\(input:focus-visible\)\s*\{([^}]*)\}/,
    ];
    for (const pattern of focusRules) {
      expect(ruleBody(pattern)).toMatch(/outline:\s*2px solid var\(--color-lume-hi\);/);
    }

    const navActiveStops = colorMixStops(
      ruleBody(/\.nav-link\[aria-current='page'\]\s*\{([^}]*)\}/),
    );
    expect(navActiveStops.length).toBeGreaterThanOrEqual(2);
    const deck = hexToRgb(token('deck'));
    const navRendered = navActiveStops.map(({ name, alpha }) =>
      compositeOver(hexToRgb(token(name)), alpha, deck),
    );

    const beamStops = colorMixStops(ruleBody(/(?:^|\n)\s*\.beam\s*\{([^}]*)\}/));
    const stage = hexToRgb(token('stage'));
    const beamRendered = beamStops.map(({ name, alpha }) =>
      compositeOver(hexToRgb(token(name)), alpha, stage),
    );

    const focusRing = hexToRgb(token('lume-hi'));
    const activeIcon = hexToRgb(token('lume'));
    const ink = hexToRgb(token('ink'));

    for (const bg of [...navRendered, ...beamRendered]) {
      expect(contrastRgb(focusRing, bg)).toBeGreaterThanOrEqual(3);
      expect(contrastRgb(ink, bg)).toBeGreaterThanOrEqual(4.5);
    }
    for (const bg of navRendered) {
      expect(contrastRgb(activeIcon, bg)).toBeGreaterThanOrEqual(3);
    }
  });

  it('paints the window the same night before the app loads', () => {
    expect(tauriConfig.app.windows[0].backgroundColor).toBe(token('night'));
    expect(indexHtml).toContain(`name="theme-color" content="${token('night')}"`);
  });
});
