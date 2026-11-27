import { describe, expect, it } from 'vitest';
import { countLabel, formatDuration, formatNumber, formatTotalDuration } from './format';

describe('formatDuration', () => {
  it.each([
    [0, '0:00'],
    [9_000, '0:09'],
    [201_000, '3:21'],
    [201_999, '3:21'],
    [600_000, '10:00'],
    [3_723_000, '1:02:03'],
  ])('shows %i ms as %s', (ms, shown) => {
    expect(formatDuration(ms)).toBe(shown);
  });
});

describe('formatTotalDuration', () => {
  it.each([
    [201_000, '3 min'],
    [3_599_000, '1 hr'],
    [3_600_000, '1 hr'],
    [3_720_000, '1 hr 2 min'],
    [1_005_000_000, '279 hr 10 min'],
    [4_320_000_000, '1,200 hr'],
    [20_000, '1 min'],
  ])('shows %i ms as %s, never breaking a number from its unit', (ms, shown) => {
    expect(formatTotalDuration(ms).replaceAll(' ', '_')).toBe(
      shown.replace(/(\d) /g, '$1_'),
    );
  });
});

describe('formatNumber', () => {
  it('groups thousands', () => {
    expect(formatNumber(5000)).toBe('5,000');
  });
});

describe('countLabel', () => {
  it('uses the singular for exactly one', () => {
    expect(countLabel(1, 'track', 'tracks')).toBe('1 track');
    expect(countLabel(0, 'track', 'tracks')).toBe('0 tracks');
    expect(countLabel(2, 'track', 'tracks')).toBe('2 tracks');
  });
});
