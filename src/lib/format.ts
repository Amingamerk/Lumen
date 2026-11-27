/** A track length as `3:21`, or `1:02:03` from an hour up. */
export function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = String(total % 60).padStart(2, '0');
  return hours
    ? `${hours}:${String(minutes).padStart(2, '0')}:${seconds}`
    : `${minutes}:${seconds}`;
}

const numbers = new Intl.NumberFormat('en');

export function formatNumber(value: number): string {
  return numbers.format(value);
}

/** The length of a whole list as `45 min` or `279 hr 10 min`, to the minute. */
export function formatTotalDuration(ms: number): string {
  const total = Math.max(1, Math.round(ms / 60_000));
  const hours = Math.floor(total / 60);
  const minutes = total % 60;
  // A number never wraps away from its unit.
  return [
    hours ? `${formatNumber(hours)}\u00a0hr` : '',
    minutes ? `${minutes}\u00a0min` : '',
  ]
    .filter(Boolean)
    .join(' ');
}

export function countLabel(count: number, one: string, many: string): string {
  return `${numbers.format(count)} ${count === 1 ? one : many}`;
}
