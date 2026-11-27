import { describe, expect, it } from 'vitest';
import { dropGap, isSamePlace } from './reorder';

describe('reorder', () => {
  it('finds the gap and clamps', () => {
    // Rows of 56 px under a list whose top is at 100.
    expect(dropGap(100 + 10, 100, 56, 5)).toBe(0);
    expect(dropGap(100 + 40, 100, 56, 5)).toBe(1);
    expect(dropGap(100 + 56 + 10, 100, 56, 5)).toBe(1);
    expect(dropGap(100 - 500, 100, 56, 5)).toBe(0);
    expect(dropGap(100 + 5000, 100, 56, 5)).toBe(5);
  });

  it('knows a drop in place', () => {
    expect(isSamePlace(2, 2)).toBe(true);
    expect(isSamePlace(2, 3)).toBe(true);
    expect(isSamePlace(2, 1)).toBe(false);
    expect(isSamePlace(2, 4)).toBe(false);
  });
});
