import { describeError } from '../errors';
import type { Bridge, PlayerPosition, PlayerState } from '../ipc';

/**
 * A read-only mirror of the player Rust owns. A command changes nothing here:
 * the answer arrives as an event.
 */
export class PlayerStore {
  state = $state.raw<PlayerState | null>(null);
  /** The clock, which moves without a new `state`. */
  positionMs = $state.raw(0);
  /** Why the last request to Rust failed, if it did. */
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;
  // `state` again, outside the reactive graph, for ordering what arrives.
  #revision = -1;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  /** Starts mirroring. Resolves with a function that stops it. */
  async connect(): Promise<() => void> {
    const stops: (() => void)[] = [];
    try {
      // Listen first, so a change made while the snapshot is in flight still arrives.
      stops.push(await this.#bridge.onPlayerChanged((state) => this.#apply(state)));
      stops.push(await this.#bridge.onPlayerPosition((position) => this.#tick(position)));
      this.#apply(await this.#bridge.getPlayerState());
    } catch (error) {
      this.error = describeError(error);
    }
    return () => stops.forEach((stop) => stop());
  }

  playCollection(id: string, index: number) {
    return this.#send(() => this.#bridge.playCollection(id, index));
  }

  playSearchResult(trackId: string) {
    return this.#send(() => this.#bridge.playSearchResult(trackId));
  }

  togglePlay() {
    return this.#send(() => this.#bridge.togglePlay());
  }

  next() {
    return this.#send(() => this.#bridge.nextTrack());
  }

  previous() {
    return this.#send(() => this.#bridge.previousTrack());
  }

  seek(positionMs: number) {
    return this.#send(() => this.#bridge.seek(positionMs));
  }

  seekBy(deltaMs: number) {
    return this.#send(() => this.#bridge.seekBy(deltaMs));
  }

  setVolume(volume: number) {
    return this.#send(() => this.#bridge.setVolume(volume));
  }

  changeVolume(delta: number) {
    return this.#send(() => this.#bridge.changeVolume(delta));
  }

  /** Whether Rust accepted the move. */
  moveQueueEntry(key: number, before: number | null) {
    return this.#send(() => this.#bridge.moveQueueEntry(key, before));
  }

  async #send(command: () => Promise<void>): Promise<boolean> {
    try {
      await command();
      this.error = null;
      return true;
    } catch (error) {
      this.error = describeError(error);
      return false;
    }
  }

  #apply(next: PlayerState) {
    if (next.revision < this.#revision) return;
    this.#revision = next.revision;
    this.state = next;
    this.positionMs = next.positionMs;
  }

  #tick(position: PlayerPosition) {
    if (position.revision === this.#revision) this.positionMs = position.positionMs;
  }
}
