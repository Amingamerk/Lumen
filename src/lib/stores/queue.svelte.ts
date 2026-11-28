import { describeError } from '../errors';
import type { Bridge, QueueSlice } from '../ipc';

/** The upcoming rows the queue panel shows. The whole Queue stays in Rust. */
export class QueueStore {
  /** The last window Rust sent. It stays until a newer one lands. */
  slice = $state.raw<QueueSlice | null>(null);
  /** The same for the entries played before, the latest first. */
  played = $state.raw<QueueSlice | null>(null);
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;
  // Answers can arrive out of order; only the latest request counts.
  #requests = 0;
  #playedRequests = 0;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  // Called from an effect, so it must not read state it writes.
  async load(offset: number, limit: number): Promise<void> {
    const ticket = ++this.#requests;
    try {
      const slice = await this.#bridge.getQueue(offset, limit);
      if (ticket !== this.#requests) return;
      this.slice = slice;
      this.error = null;
    } catch (error) {
      if (ticket === this.#requests) this.error = describeError(error);
    }
  }

  /** The same for the entries played before the current one. */
  async loadPlayed(offset: number, limit: number): Promise<void> {
    const ticket = ++this.#playedRequests;
    try {
      const slice = await this.#bridge.getQueueHistory(offset, limit);
      if (ticket !== this.#playedRequests) return;
      this.played = slice;
      this.error = null;
    } catch (error) {
      if (ticket === this.#playedRequests) this.error = describeError(error);
    }
  }
}
