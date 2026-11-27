/**
 * Runs one send at a time. Values that arrive meanwhile replace each other, so
 * only the newest follows. Every call resolves once nothing is left to send.
 */
export function latestOnly<T>(send: (value: T) => Promise<void>): (value: T) => Promise<void> {
  let running: Promise<void> | null = null;
  let waiting: { value: T } | null = null;

  async function drain() {
    try {
      while (waiting) {
        const { value } = waiting;
        waiting = null;
        await send(value);
      }
    } finally {
      running = null;
    }
  }

  return (value) => {
    waiting = { value };
    return (running ??= drain());
  };
}
