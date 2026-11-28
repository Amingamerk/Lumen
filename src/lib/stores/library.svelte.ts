import { SvelteSet } from 'svelte/reactivity';
import { describeError } from '../errors';
import type { Bridge, LibrarySummary, PlayList, TrackRow, TrackSlice } from '../ipc';
import { TrackWindow } from './trackWindow.svelte';

/**
 * A mirror of the liked songs and the history Rust keeps. Which songs are liked
 * is learned for the ones on show only; the lists stay in Rust and come as
 * slices.
 */
export class LibraryStore {
  summary = $state.raw<LibrarySummary | null>(null);
  /** Rises when a like or a play changes what the lists hold. */
  revision = $state.raw(0);
  /** Why the last request to Rust failed, if it did. */
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;
  readonly #liked = new SvelteSet<string>();
  /** Songs whose like is known, so they are not asked about again. */
  readonly #known = new SvelteSet<string>();
  /** The newest library event heard of. */
  #seen = 0;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  /** Starts mirroring. Resolves with a function that stops it. */
  async connect(): Promise<() => void> {
    const stops: (() => void)[] = [];
    try {
      // Listen first, so a change made while the summary is in flight still arrives.
      stops.push(await this.#bridge.onLikesChanged((change) => this.#likes(change)));
      stops.push(
        await this.#bridge.onHistoryChanged((change) => {
          if (!this.#fresh(change.revision)) return;
          if (this.summary) this.summary = { ...this.summary, playedCount: change.playedCount };
          this.revision += 1;
        }),
      );
      const summary = await this.#bridge.getLibrarySummary();
      // A summary older than an event already heard is not newer news.
      if (summary.revision >= this.#seen) {
        this.summary = summary;
        this.#seen = summary.revision;
      }
    } catch (error) {
      this.error = describeError(error);
    }
    return () => stops.forEach((stop) => stop());
  }

  isLiked(trackId: string): boolean {
    return this.#liked.has(trackId);
  }

  /** Learns which of the songs on show are liked. */
  async watch(trackIds: string[]): Promise<void> {
    const unknown = trackIds.filter((id) => !this.#known.has(id));
    if (unknown.length === 0) return;
    try {
      const liked = await this.#bridge.getLikedIds(unknown);
      for (const id of unknown) this.#known.add(id);
      for (const id of liked) this.#liked.add(id);
    } catch (error) {
      this.error = describeError(error);
    }
  }

  /** Likes or unlikes. The change shows when Rust announces it. */
  toggle(trackId: string): Promise<void> {
    return this.#send(() =>
      this.isLiked(trackId)
        ? this.#bridge.unlikeTrack(trackId)
        : this.#bridge.likeTrack(trackId),
    );
  }

  /** Queues the whole list and plays from one of its rows. */
  play(list: PlayList, position: number, trackId: string): Promise<void> {
    return this.#send(() => this.#bridge.playList(list, position, trackId));
  }

  /** A new window onto the liked songs. */
  likedWindow(): TrackWindow {
    return this.#window(
      (offset, limit) => this.#bridge.getLikedSlice(offset, limit),
      (held) => {
        // Every row of this list is liked.
        for (const { id } of held) {
          this.#known.add(id);
          this.#liked.add(id);
        }
      },
    );
  }

  /** A new window onto the songs played, the latest first. */
  recentWindow(): TrackWindow {
    return this.#window(
      (offset, limit) => this.#bridge.getRecentSlice(offset, limit),
      (held) => void this.watch(held.map((row) => row.id)),
    );
  }

  #window(
    load: (offset: number, limit: number) => Promise<TrackSlice>,
    learn: (held: TrackRow[]) => void,
  ): TrackWindow {
    return new TrackWindow(async (offset, limit) => {
      try {
        const slice = await load(offset, limit);
        this.error = null;
        learn(slice.tracks);
        return slice;
      } catch (error) {
        this.error = describeError(error);
        throw error;
      }
    });
  }

  #likes(change: { revision: number; trackId: string; liked: boolean; likedCount: number }) {
    if (!this.#fresh(change.revision)) return;
    this.#known.add(change.trackId);
    if (change.liked) this.#liked.add(change.trackId);
    else this.#liked.delete(change.trackId);
    if (this.summary) this.summary = { ...this.summary, likedCount: change.likedCount };
    this.revision += 1;
  }

  /** Whether an event is newer than any heard of. */
  #fresh(revision: number): boolean {
    if (revision <= this.#seen) return false;
    this.#seen = revision;
    return true;
  }

  async #send(command: () => Promise<void>): Promise<void> {
    try {
      await command();
      this.error = null;
    } catch (error) {
      this.error = describeError(error);
    }
  }
}
