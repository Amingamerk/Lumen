import { describeError } from '../errors';
import type { Bridge, SearchKind, SearchView, Shelf, SourceFilter } from '../ipc';

/**
 * A mirror of the Home shelves and the search Rust builds. Each view asks when it
 * opens, and the search also follows its events. No searching or filtering happens here.
 */
export class CatalogStore {
  home = $state.raw<Shelf[] | null>(null);
  search = $state.raw<SearchView | null>(null);
  /** Why the last request to Rust failed, if it did. */
  error = $state.raw<string | null>(null);

  readonly #bridge: Bridge;

  constructor(bridge: Bridge) {
    this.#bridge = bridge;
  }

  /** Starts mirroring the search. Resolves with a function that stops it. */
  async connect(): Promise<() => void> {
    let stop = () => {};
    try {
      stop = await this.#bridge.onSearchChanged((view) => this.#applySearch(view));
    } catch (error) {
      this.error = describeError(error);
    }
    return () => stop();
  }

  loadHome(): Promise<void> {
    return this.#ask(
      () => this.#bridge.getHome(),
      (home) => (this.home = home),
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
