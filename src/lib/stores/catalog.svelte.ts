import { describeError } from '../errors';
import type { Bridge, CollectionDetail, Library, SearchView, Shelf, SourceFilter } from '../ipc';
import { TrackWindow } from './trackWindow.svelte';

/**
 * A mirror of the catalog views Rust builds. Each view asks when it opens; no
 * searching or filtering happens on this side.
 */
export class CatalogStore {
  home = $state.raw<Shelf[] | null>(null);
  library = $state.raw<Library | null>(null);
  search = $state.raw<SearchView | null>(null);
  /** The playlist or album asked for last. */
  collection = $state.raw<CollectionDetail | null>(null);
  /** Why the last request to Rust failed, if it did. */
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;
  // Answers can arrive out of order; only the latest request of each kind counts.
  #searches = 0;
  #collections = 0;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  loadHome(): Promise<void> {
    return this.#ask(
      () => this.#bridge.getHome(),
      (home) => (this.home = home),
    );
  }

  loadLibrary(): Promise<void> {
    return this.#ask(
      () => this.#bridge.getLibrary(),
      (library) => (this.library = library),
    );
  }

  loadCollection(id: string): Promise<void> {
    const ticket = ++this.#collections;
    return this.#ask(
      () => this.#bridge.getCollection(id),
      (collection) => (this.collection = collection),
      () => ticket === this.#collections,
    );
  }

  /** A new window onto one Collection's Tracks, for one visit to its page. */
  tracksOf(id: string): TrackWindow {
    return new TrackWindow(async (offset, limit) => {
      try {
        const slice = await this.#bridge.getTrackSlice(id, offset, limit);
        this.error = null;
        return slice;
      } catch (error) {
        this.error = describeError(error);
        throw error;
      }
    });
  }

  /** Fetches the search Rust remembers. */
  openSearch(): Promise<void> {
    return this.#askSearch(() => this.#bridge.getSearch());
  }

  runSearch(query: string, filter: SourceFilter): Promise<void> {
    return this.#askSearch(() => this.#bridge.searchCatalog(query, filter));
  }

  #askSearch(request: () => Promise<SearchView>): Promise<void> {
    const ticket = ++this.#searches;
    return this.#ask(
      request,
      (search) => (this.search = search),
      () => ticket === this.#searches,
    );
  }

  async #ask<T>(
    request: () => Promise<T>,
    keep: (answer: T) => void,
    isLatest: () => boolean = () => true,
  ): Promise<void> {
    try {
      const answer = await request();
      if (!isLatest()) return;
      keep(answer);
      this.error = null;
    } catch (error) {
      if (isLatest()) this.error = describeError(error);
    }
  }
}
