import { describe, expect, it } from 'vitest';
import { pagesCovering, rowRange } from './virtual';

/** A 5,000-row list with 56 px rows in a viewport that fits ten of them. */
const range = (scrolled: number, { viewport = 560, total = 5000, overscan = 8 } = {}) =>
  rowRange({ scrolled, viewport, rowHeight: 56, total, overscan });

describe('rowRange', () => {
  it('starts with the rows in view plus the overscan below', () => {
    expect(range(0)).toEqual({ start: 0, end: 18 });
  });

  it('keeps the overscan on both sides in the middle', () => {
    expect(range(2500 * 56)).toEqual({ start: 2492, end: 2518 });
  });

  it('counts rows that are only partly in view', () => {
    expect(range(100, { viewport: 100, overscan: 0 })).toEqual({ start: 1, end: 4 });
  });

  it('stops at the last row', () => {
    expect(range(5000 * 56 - 560)).toEqual({ start: 4982, end: 5000 });
  });

  it('never goes past the end, however far the page scrolls', () => {
    expect(range(10_000_000)).toEqual({ start: 5000, end: 5000 });
  });

  it('draws only what shows while a header still sits above the list', () => {
    expect(range(-300, { viewport: 600 })).toEqual({ start: 0, end: 14 });
  });

  it('draws nothing while the list is far below the viewport', () => {
    expect(range(-2000, { viewport: 600 })).toEqual({ start: 0, end: 0 });
  });

  it('draws nothing for an empty list', () => {
    expect(range(0, { total: 0 })).toEqual({ start: 0, end: 0 });
  });

  it('draws a short list whole', () => {
    expect(range(0, { total: 3 })).toEqual({ start: 0, end: 3 });
  });

  it('is never larger than the viewport plus the overscan, wherever it is', () => {
    const most = Math.ceil(560 / 56) + 1 + 2 * 8;

    for (let scrolled = -600; scrolled <= 5000 * 56; scrolled += 997) {
      const { start, end } = range(scrolled);

      expect(end - start).toBeLessThanOrEqual(most);
    }
  });
});

describe('pagesCovering', () => {
  it.each([
    [0, 18, [0]],
    [40, 60, [0, 1]],
    [49, 50, [0]],
    [50, 51, [1]],
    [90, 210, [1, 2, 3, 4]],
    [4999, 5000, [99]],
    [0, 0, []],
    [5000, 5000, []],
  ])('rows %i to %i sit on pages %j', (start, end, pages) => {
    expect(pagesCovering({ start, end }, 50)).toEqual(pages);
  });
});
