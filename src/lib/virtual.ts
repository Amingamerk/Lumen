// Which rows of a long list to draw. Every row has the same height.

/** Rows from `start` up to, not including, `end`. Positions count from 0. */
export interface RowRange {
  start: number;
  end: number;
}

interface View {
  /** How far the list's top is above the viewport's; negative while it is below. */
  scrolled: number;
  viewport: number;
  rowHeight: number;
  total: number;
  /** Spare rows drawn on each side, so a quick scroll does not show a gap. */
  overscan: number;
}

/** The rows in view, plus the overscan, within the list. */
export function rowRange({ scrolled, viewport, rowHeight, total, overscan }: View): RowRange {
  const within = (row: number) => Math.min(Math.max(row, 0), total);
  return {
    start: within(Math.floor(scrolled / rowHeight) - overscan),
    end: within(Math.ceil((scrolled + viewport) / rowHeight) + overscan),
  };
}

/** The pages of `pageSize` rows that hold any row of the range, counted from 0. */
export function pagesCovering({ start, end }: RowRange, pageSize: number): number[] {
  if (end <= start) return [];
  const first = Math.floor(start / pageSize);
  const last = Math.floor((end - 1) / pageSize);
  return Array.from({ length: last - first + 1 }, (_, at) => first + at);
}
