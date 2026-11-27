import { SvelteMap } from 'svelte/reactivity';
import type { TrackRow, TrackSlice } from '../ipc';
import { pagesCovering } from '../virtual';

/** Rows asked for at a time. Rust caps a slice at 200. */
export const PAGE_SIZE = 50;
/** Pages kept for scrolling back. More are held only if one window needs them. */
export const MAX_PAGES = 6;

/**
 * The part of one Collection's Tracks the UI holds. The whole list stays in Rust;
 * this asks for the pages the list is showing and keeps the last few.
 */
export class TrackWindow {
  /** How many Tracks the Collection has, once Rust has said. */
  total = $state.raw<number | null>(null);

  readonly #load: (offset: number, limit: number) => Promise<TrackSlice>;
  readonly #pages = new SvelteMap<number, TrackRow[]>();
  /** The numbers of the held pages, the longest unused first. */
  #used: number[] = [];
  #pending: number[] = [];
  #wanted: number[] = [];

  constructor(load: (offset: number, limit: number) => Promise<TrackSlice>) {
    this.#load = load;
  }

  /** The Track at this position, if its page is here. */
  row(position: number): TrackRow | undefined {
    return this.#pages.get(Math.floor(position / PAGE_SIZE))?.[position % PAGE_SIZE];
  }

  /**
   * Makes sure the rows from `start` up to `end` are here or on their way. Cheap
   * to call on every scroll: nothing happens until the window reaches other pages.
   */
  show(start: number, end: number): void {
    const total = this.total;
    // The first page also says how long the list is.
    const wanted =
      total === null ? [0] : pagesCovering({ start, end: Math.min(end, total) }, PAGE_SIZE);
    if (wanted.join() === this.#wanted.join()) return;
    this.#wanted = wanted;

    for (const page of wanted) {
      if (this.#used.includes(page)) this.#touch(page);
      else if (!this.#pending.includes(page)) void this.#fetch(page);
    }
  }

  async #fetch(page: number) {
    this.#pending.push(page);
    try {
      const slice = await this.#load(page * PAGE_SIZE, PAGE_SIZE);
      this.#keep(page, slice.tracks);
      this.total = slice.total;
    } catch {
      // The loader reports it. The page is asked for again when the window next
      // moves onto it, never in a loop.
    } finally {
      this.#pending = this.#pending.filter((other) => other !== page);
    }
  }

  #touch(page: number) {
    this.#used = [...this.#used.filter((other) => other !== page), page];
  }

  #keep(page: number, held: TrackRow[]) {
    this.#pages.set(page, held);
    this.#touch(page);
    while (this.#used.length > MAX_PAGES) {
      const spare = this.#used.find((other) => !this.#wanted.includes(other));
      if (spare === undefined) break;
      this.#used = this.#used.filter((other) => other !== spare);
      this.#pages.delete(spare);
    }
  }
}
