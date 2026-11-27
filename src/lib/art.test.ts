import { describe, expect, it } from 'vitest';
import { placeholderArt } from './art';

describe('placeholderArt', () => {
  it.each([
    ['Night shift', 'NS'],
    ['Antenna', 'A'],
    ['Songs for the long drive home', 'SF'],
    ['søren vale', 'SV'],
    ['  (Live)   at the pier ', 'LA'],
    ['!!!', ''],
  ])('takes initials from %j', (title, initials) => {
    expect(placeholderArt(title).initials).toBe(initials);
  });

  it('gives a title the same cover every time', () => {
    expect(placeholderArt('Low Tide')).toEqual(placeholderArt('Low Tide'));
  });

  it('varies the covers between titles, within the styled range', () => {
    const covers = ['Low Tide', 'Crossings', 'Paper Weather', 'Static Bloom', 'Small Hours'].map(
      placeholderArt,
    );

    for (const { tone, light } of covers) {
      expect([0, 1, 2, 3, 4, 5]).toContain(tone);
      expect([0, 1, 2, 3]).toContain(light);
    }
    expect(new Set(covers.map(({ tone }) => tone)).size).toBeGreaterThan(1);
  });
});
