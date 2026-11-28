import { describeError } from '../errors';
import type { Bridge, HomeView, SearchKind, SearchView, SourceFilter } from '../ipc';

/**
 * A mirror of the Home shelves (kept fresh by the library events) and the search Rust builds. Each view asks when it
 * opens, and the search also follows its events. No searching or filtering happens here.
 */
export class CatalogStore {
  home = $state.raw<HomeView | null>(null);
  search = $state.raw<SearchView | null>(null);
  /** Why the last request to Rust failed, if it did. */
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;
  #homeAsks = 0;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  /** Starts mirroring the search and what changes Home. Resolves with a function that stops it. */
  async connect(): Promise<() => void> {
    const stops: (() => void)[] = [];
    try {
      stops.push(await this.#bridge.onSearchChanged((view) => this.#applySearch(view)));
      // Once Home has been shown, a like, a play or a playlist change asks again.
      const refresh = () => {
        if (this.home) void this.loadHome();
      };
      stops.push(await this.#bridge.onLikesChanged(refresh));
      stops.push(await this.#bridge.onHistoryChanged(refresh));
      stops.push(await this.#bridge.onPlaylistsChanged(refresh));
    } catch (error) {
      this.error = describeError(error);
    }
    return () => stops.forEach((stop) => stop());
  }

  loadHome(): Promise<void> {
    const ask = ++this.#homeAsks;
    return this.#ask(
      () => this.#bridge.getHome(),
      // An answer to an older question does not replace a newer one.
      (home) => {
        if (ask === this.#homeAsks) this.home = home;
      },
    );
  }

  /** Fetches the search Rust remembers. */
  openSearch(): Promise<void> {
    return this.#ask(
      () => this.#bridge.getSearch(),
      (view) => this.#applySearch(view),
    );
  }

  submitSearch(query: string, filter: SourceFilter): Promise<void> {
    return this.#ask(
      () => this.#bridge.submitSearch(query, filter),
      () => {},
    );
  }

  setSearchFilter(filter: SourceFilter): Promise<void> {
    return this.#ask(
      () => this.#bridge.setSearchFilter(filter),
      () => {},
    );
  }

  /** The type chip. Rust shows what is known or searches; nothing is asked from here. */
  setSearchKind(kind: SearchKind): Promise<void> {
    return this.#ask(
      () => this.#bridge.setSearchKind(kind),
      () => {},
    );
  }

  /** Events and answers can overtake each other; the revision tells them apart. */
  #applySearch(view: SearchView) {
    if (view.revision >= (this.search?.revision ?? 0)) this.search = view;
  }

  async #ask<T>(request: () => Promise<T>, keep: (answer: T) => void): Promise<void> {
    try {
      keep(await request());
      this.error = null;
    } catch (error) {
      this.error = describeError(error);
    }
  }
}
