import { describe, expect, it } from 'vitest';
import type { TrackSlice } from '../ipc';
import { deferred, trackSlice } from '../testing';
import { MAX_PAGES, PAGE_SIZE, TrackWindow } from './trackWindow.svelte';

/** A window over a list of `total` rows that records what it asks for. */
function setUp(total = 5000) {
  const asked: number[] = [];
  const answers = {
    next: (offset: number, limit: number): TrackSlice | Promise<TrackSlice> =>
      trackSlice(offset, limit, total),
  };
  const tracks = new TrackWindow(async (offset, limit) => {
    expect(limit).toBe(PAGE_SIZE);
    asked.push(offset);
    return answers.next(offset, limit);
  });
  /** Shows a window and lets the answers arrive. */
  const show = async (start: number, end = start + 20) => {
    tracks.show(start, end);
    await settled();
  };
  return { tracks, asked, answers, show };
}

const settled = () => new Promise((done) => setTimeout(done));

describe('TrackWindow', () => {
  it('knows nothing before a window is shown', () => {
    const { tracks, asked } = setUp();

    expect([tracks.total, tracks.row(0)]).toEqual([null, undefined]);
    expect(asked).toEqual([]);
  });

  it('asks for the first page to learn how long the list is', async () => {
    const { tracks, asked, show } = setUp();

    await show(0, 0);

    expect(asked).toEqual([0]);
    expect(tracks.total).toBe(5000);
    expect(tracks.row(0)?.title).toBe('Track 1');
    expect(tracks.row(PAGE_SIZE - 1)?.title).toBe(`Track ${PAGE_SIZE}`);
    expect(tracks.row(PAGE_SIZE)).toBeUndefined();
  });

  it('asks only for the pages the window covers', async () => {
    const { tracks, asked, show } = setUp();
    await show(0);

    await show(4980, 5000);

    expect(asked).toEqual([0, 4950]);
    expect(tracks.row(4999)?.title).toBe('Track 5000');
    expect(tracks.row(2500)).toBeUndefined();
  });

  it('fetches both pages of a window that straddles them', async () => {
    const { asked, show } = setUp();
    await show(0);

    await show(PAGE_SIZE * 3 - 5);

    expect(asked).toEqual([0, 100, 150]);
  });

  it('does not ask again while the window stays on the same pages', async () => {
    const { asked, show } = setUp();
    await show(0);

    for (let start = 0; start < 25; start++) await show(start);

    expect(asked).toEqual([0]);
  });

  it('does not ask again for a page that is still on its way', async () => {
    const { tracks, asked, answers, show } = setUp();
    await show(0);
    const slow = deferred<TrackSlice>();
    answers.next = () => slow.promise;

    tracks.show(100, 120);
    tracks.show(140, 160);
    tracks.show(100, 120);

    expect(asked).toEqual([0, 100, 150]);
  });

  it('keeps a page it has when the window comes back', async () => {
    const { asked, show } = setUp();
    await show(0);
    await show(200);

    await show(0);

    expect(asked).toEqual([0, 200]);
  });

  it('holds a bounded number of pages and drops the one unused longest', async () => {
    const { tracks, asked, show } = setUp();
    for (let page = 0; page < MAX_PAGES; page++) await show(page * PAGE_SIZE);
    await show(0);

    await show(MAX_PAGES * PAGE_SIZE);

    const held = Array.from({ length: 100 }, (_, page) => page).filter((page) =>
      tracks.row(page * PAGE_SIZE),
    );
    expect(held).toHaveLength(MAX_PAGES);
    expect(held).toContain(0);
    expect(held).not.toContain(1);

    await show(PAGE_SIZE);
    expect(asked.filter((offset) => offset === PAGE_SIZE)).toHaveLength(2);
  });

  it('never drops a page the current window needs', async () => {
    const { tracks, show } = setUp();
    await show(0);

    await show(0, (MAX_PAGES + 2) * PAGE_SIZE);

    expect(tracks.row(0)).toBeDefined();
    expect(tracks.row((MAX_PAGES + 2) * PAGE_SIZE - 1)).toBeDefined();
  });

  it('does not retry a failed page until the window moves away and back', async () => {
    const { tracks, asked, answers, show } = setUp();
    await show(0);
    answers.next = () => Promise.reject('catalog unavailable');
    await show(100);
    await show(101);
    expect(asked).toEqual([0, 100]);
    expect(tracks.row(100)).toBeUndefined();

    answers.next = (offset, limit) => trackSlice(offset, limit, 5000);
    await show(0);
    await show(100);

    expect(asked).toEqual([0, 100, 100]);
    expect(tracks.row(100)?.title).toBe('Track 101');
  });

  it('asks for nothing more of an empty list', async () => {
    const { tracks, asked, show } = setUp(0);

    await show(0);
    await show(0, 40);

    expect(asked).toEqual([0]);
    expect(tracks.total).toBe(0);
  });
});
