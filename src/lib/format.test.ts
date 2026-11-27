import { describe, expect, it } from 'vitest';
import { countLabel, formatDuration } from './format';

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

describe('countLabel', () => {
  it('uses the singular for exactly one', () => {
    expect(countLabel(1, 'track', 'tracks')).toBe('1 track');
    expect(countLabel(0, 'track', 'tracks')).toBe('0 tracks');
    expect(countLabel(2, 'track', 'tracks')).toBe('2 tracks');
  });
});
