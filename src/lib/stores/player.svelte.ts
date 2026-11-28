import { describeError } from '../errors';
import type { Bridge, PlayerPosition, PlayerState, QueuePlacement } from '../ipc';

/** Something to tell the user. A new object each time it should be said. */
export interface Notice {
  message: string;
  /** A refused command is over with the next one; a failure with the failed state. */
  from: 'command' | 'playback';
}

/**
 * A read-only mirror of the player Rust owns. A command changes nothing here:
 * the answer arrives as an event.
 */
export class PlayerStore {
  state = $state.raw<PlayerState | null>(null);
  /** The clock, which moves without a new `state`. */
  positionMs = $state.raw(0);
  /** Why mirroring could not start, if it could not. */
  error = $state.raw<string | null>(null);
  notice = $state.raw<Notice | null>(null);

  readonly #bridge: Bridge;
  // `state` again, outside the reactive graph, for ordering what arrives.
  #revision = -1;
  /** The playback failure already given notice of. */
  #failure: string | null = null;
  /** The skipped song already given notice of. */
  #skipped: number | null = null;

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

  playSearchResult(trackId: string) {
    return this.#send(() => this.#bridge.playSearchResult(trackId));
  }

  togglePlay() {
    // Trying again after a failure: its notice is over, and if it fails again,
    // say so again.
    if (this.state?.status === 'error') {
      this.#failure = null;
      if (this.notice?.from === 'playback') this.notice = null;
    }
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

  /** Adds a song to the Queue. Rust answers with an event. */
  queueTrack(trackId: string, placement: QueuePlacement) {
    return this.#send(() => this.#bridge.queueTrack(trackId, placement));
  }

  removeQueueEntry(key: number) {
    return this.#send(() => this.#bridge.removeQueueEntry(key));
  }

  clearQueue() {
    return this.#send(() => this.#bridge.clearQueue());
  }

  toggleShuffle() {
    return this.#send(() => this.#bridge.toggleShuffle());
  }

  cycleRepeat() {
    return this.#send(() => this.#bridge.cycleRepeat());
  }

  dismissNotice() {
    this.notice = null;
  }

  async #send(command: () => Promise<void>): Promise<boolean> {
    try {
      await command();
      this.error = null;
      if (this.notice?.from === 'command') this.notice = null;
      return true;
    } catch (error) {
      this.notice = { message: describeError(error), from: 'command' };
      return false;
    }
  }

  #apply(next: PlayerState) {
    if (next.revision < this.#revision) return;
    this.#revision = next.revision;
    this.state = next;
    this.positionMs = next.positionMs;
    this.#noticeFailure(next);
    this.#noticeSkip(next);
  }

  #noticeSkip({ skipped }: PlayerState) {
    if (skipped === null || skipped.key === this.#skipped) return;
    this.#skipped = skipped.key;
    this.notice = { message: skipped.message, from: 'playback' };
  }

  #noticeFailure({ error, current }: PlayerState) {
    const failure = error === null ? null : `${current?.key}/${error}`;
    if (failure === this.#failure) return;
    this.#failure = failure;
    if (error !== null) this.notice = { message: error, from: 'playback' };
    else if (this.notice?.from === 'playback') this.notice = null;
  }

  #tick(position: PlayerPosition) {
    if (position.revision === this.#revision) this.positionMs = position.positionMs;
  }
}
