// Where a dragged row would land. A gap is a place between rows, `0..=total`:
// gap `n` is just before row `n`.

/** The gap nearest the pointer. `listTop` is where the list's first row starts. */
export function dropGap(pointerY: number, listTop: number, rowHeight: number, total: number) {
  return Math.min(Math.max(Math.round((pointerY - listTop) / rowHeight), 0), total);
}

/** Whether dropping the row at `from` into `gap` leaves it where it is. */
export function isSamePlace(from: number, gap: number): boolean {
  return gap === from || gap === from + 1;
}
