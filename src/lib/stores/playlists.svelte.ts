import { SvelteMap } from 'svelte/reactivity';
import { describeError } from '../errors';
import type { Bridge, PlaylistsChanged, PlaylistSummary } from '../ipc';
import { TrackWindow } from './trackWindow.svelte';

/** The song a person is choosing a playlist for. */
export interface Pick {
  trackId: string;
  title: string;
}

/**
 * A mirror of the local playlists Rust keeps. The list of playlists follows its
 * events; the entries of a playlist stay in Rust and come as slices.
 */
export class PlaylistsStore {
  /** `null` until Rust has said, so "no playlists yet" is not shown too early. */
  playlists = $state.raw<PlaylistSummary[] | null>(null);
  /** The song being added to a playlist, while the choice is open. */
  picking = $state.raw<Pick | null>(null);
  /** Why the last request to Rust failed, if it did. */
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;
  /** For each playlist, the revision of the event that last changed it. */
  readonly #versions = new SvelteMap<string, number>();
  #seen = -1;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  /** Starts mirroring. Resolves with a function that stops it. */
  async connect(): Promise<() => void> {
    let stop = () => {};
    try {
      // Listen first, so a change made while the list is in flight still arrives.
      stop = await this.#bridge.onPlaylistsChanged((change) => this.#apply(change));
      this.#apply(await this.#bridge.getPlaylists());
    } catch (error) {
      this.error = describeError(error);
    }
    return () => stop();
  }

  find(id: string): PlaylistSummary | undefined {
    return this.playlists?.find((playlist) => playlist.id === id);
  }

  /** Changes whenever the playlist's name or entries do. */
  version(id: string): number {
    return this.#versions.get(id) ?? 0;
  }

  /** A new window onto one playlist's entries, for one version of it. */
  windowOf(id: string): TrackWindow {
    return new TrackWindow(async (offset, limit) => {
      try {
        const slice = await this.#bridge.getPlaylistSlice(id, offset, limit);
        this.error = null;
        return slice;
      } catch (error) {
        this.error = describeError(error);
        throw error;
      }
    });
  }

  /** Makes a playlist and says which. The list follows the event. */
  async create(name: string): Promise<PlaylistSummary | null> {
    try {
      const made = await this.#bridge.createPlaylist(name);
      this.error = null;
      return made;
    } catch (error) {
      this.error = describeError(error);
      return null;
    }
  }

  rename(id: string, name: string): Promise<void> {
    return this.#send(() => this.#bridge.renamePlaylist(id, name));
  }

  remove(id: string): Promise<void> {
    return this.#send(() => this.#bridge.deletePlaylist(id));
  }

  add(id: string, trackId: string): Promise<void> {
    return this.#send(() => this.#bridge.addToPlaylist(id, trackId));
  }

  removeEntry(id: string, position: number, trackId: string): Promise<void> {
    return this.#send(() => this.#bridge.removeFromPlaylist(id, position, trackId));
  }

  /** Opens the choice of a playlist for this song. */
  choose(pick: Pick) {
    this.picking = pick;
  }

  stopChoosing() {
    this.picking = null;
  }

  /** An older event than one already heard of is not news. */
  #apply(change: PlaylistsChanged) {
    if (change.revision < this.#seen) return;
    this.#seen = change.revision;
    this.playlists = change.playlists;
    if (change.changed !== null) this.#versions.set(change.changed, change.revision);
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
