// The queue panel, driven through the whole app against the fake Rust side.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import type { QueueSlice } from './lib/ipc';
import { appStores, fakeBridge, playerState, queueSlice } from './lib/testing';

const ROW = 56;

const panel = () => within(screen.getByRole('complementary', { name: 'Queue' }));
const items = () => panel().queryAllByRole('listitem');
const handle = (title: string) =>
  panel().getByRole<HTMLButtonElement>('button', { name: `Move ${title}` });
const list = () => panel().getByRole('list');
const moves = (bridge: ReturnType<typeof fakeBridge>) =>
  bridge.commands.filter(([name]) => name === 'moveQueueEntry');

function openQueue(bridge: ReturnType<typeof fakeBridge>, revision = 1) {
  bridge.emit({ route: { view: 'home' }, queueOpen: true, revision });
}

/** The app with the panel open on `total` upcoming entries. */
async function open(total = 5, bridge = fakeBridge()) {
  bridge.answerQueue = (offset, limit) => queueSlice(offset, limit, total);
  render(App, { props: appStores(bridge) });
  await waitFor(() => expect(bridge.calls).toContain('getPlayerState'));
  await screen.findByRole('link', { name: /^night shift, playlist$/i });
  bridge.emitPlayer(playerState({ upcoming: total }));
  openQueue(bridge);
  await screen.findByRole('complementary', { name: 'Queue' });
  if (total > 0) await panel().findByRole('button', { name: 'Move Track 1' });
  return bridge;
}

/** jsdom has no layout: the list starts at 100 px. */
function placeList() {
  list().getBoundingClientRect = () => new DOMRect(0, 100, 320, 5 * ROW);
}
const at = (row: number) => ({ clientY: 100 + row * ROW + 10, button: 0, pointerId: 1 });

describe('queue panel', () => {
  it('opens only when Rust says so, and closes through Rust', async () => {
    const bridge = fakeBridge();
    render(App, { props: appStores(bridge) });
    await screen.findByRole('link', { name: /^night shift, playlist$/i });
    const button = screen.getByRole('button', { name: 'Queue' });

    await fireEvent.click(button);
    expect(bridge.commands).toEqual([['toggleQueue']]);
    expect(screen.queryByRole('complementary', { name: 'Queue' })).toBeNull();

    openQueue(bridge);
    await screen.findByRole('complementary', { name: 'Queue' });
    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(document.querySelector('.shell')?.getAttribute('data-queue')).toBe('open');

    await fireEvent.click(panel().getByRole('button', { name: 'Close queue' }));
    expect(bridge.commands).toEqual([['toggleQueue'], ['toggleQueue']]);
  });

  it('shows what is playing apart from what comes next', async () => {
    await open(3);

    expect(panel().getByRole('heading', { name: 'Now playing' })).toBeTruthy();
    expect(panel().getByText('Harbor Lights')).toBeTruthy();
    expect(panel().getByRole('heading', { name: 'Next up' })).toBeTruthy();
    expect(items()).toHaveLength(3);
  });

  it('says so when nothing is coming', async () => {
    await open(0);

    expect(panel().getByText('Nothing up next')).toBeTruthy();
  });

  it('renders bounded rows for 5,000 upcoming', async () => {
    const bridge = await open(4999);

    expect(items().length).toBeLessThan(40);
    expect(items()[0].getAttribute('aria-setsize')).toBe('4999');
    expect(items()[0].getAttribute('aria-posinset')).toBe('1');
    for (const { limit } of bridge.queueRequests) expect(limit).toBeLessThan(40);
  });

  it('draws the rows scrolled to', async () => {
    await open(4999);
    const scroller = list().parentElement as HTMLElement;
    Object.defineProperty(scroller, 'clientHeight', { value: 10 * ROW, configurable: true });
    scroller.getBoundingClientRect = () => new DOMRect(0, 100);
    list().getBoundingClientRect = () => new DOMRect(0, 100 - 2000 * ROW);

    await fireEvent.scroll(scroller);

    expect(await panel().findByRole('button', { name: 'Move Track 2001' })).toBeTruthy();
    expect(items().length).toBeLessThan(40);
  });

  it('refetches when the queue changes, not when the clock moves', async () => {
    const bridge = await open(5);
    const before = bridge.queueRequests.length;

    bridge.emitPosition({ revision: 1, positionMs: 5000 });
    bridge.emitPlayer(playerState({ revision: 2, upcoming: 5, volume: 10 }));
    await Promise.resolve();
    expect(bridge.queueRequests).toHaveLength(before);

    bridge.emitPlayer(playerState({ revision: 3, upcoming: 5, queueRevision: 2 }));
    await waitFor(() => expect(bridge.queueRequests).toHaveLength(before + 1));
  });

  it('labels the picks of the recommender', async () => {
    const bridge = fakeBridge();
    bridge.answerQueue = (offset, limit): QueueSlice => {
      const slice = queueSlice(offset, limit, 5);
      // Entries 3 and 4 are picks; a collection row was moved below them.
      for (const [index, item] of slice.items.entries()) {
        item.autoplay = [2, 3].includes(slice.offset + index);
      }
      return slice;
    };
    render(App, { props: appStores(bridge) });
    await screen.findByRole('link', { name: /^night shift, playlist$/i });
    bridge.emitPlayer(playerState({ upcoming: 5 }));
    openQueue(bridge);
    await screen.findByRole('complementary', { name: 'Queue' });
    await panel().findByRole('button', { name: 'Move Track 1' });

    const labelled = items().map((item) => item.textContent ?? '');
    const picks = labelled.map((text) => text.includes('Up next from recommendations'));
    expect(picks).toEqual([false, false, true, false, false]);
    // Where the picks end, the list says it is back to the queue.
    expect(labelled.map((text) => text.includes('Back to your queue'))).toEqual([
      false,
      false,
      false,
      false,
      true,
    ]);
  });

  it('does not repeat the label when the window starts inside the picks', async () => {
    const bridge = fakeBridge();
    bridge.answerQueue = (offset, limit) => {
      const slice = queueSlice(offset, limit, 100);
      for (const item of slice.items) item.autoplay = true;
      return slice;
    };
    render(App, { props: appStores(bridge) });
    await screen.findByRole('link', { name: /^night shift, playlist$/i });
    bridge.emitPlayer(playerState({ upcoming: 100 }));
    openQueue(bridge);
    await screen.findByRole('complementary', { name: 'Queue' });
    await panel().findByRole('button', { name: 'Move Track 1' });
    const scroller = list().parentElement as HTMLElement;
    Object.defineProperty(scroller, 'clientHeight', { value: 5 * ROW, configurable: true });
    scroller.getBoundingClientRect = () => new DOMRect(0, 100);
    list().getBoundingClientRect = () => new DOMRect(0, 100 - 50 * ROW);

    await fireEvent.scroll(scroller);
    await panel().findByRole('button', { name: 'Move Track 51' });

    expect(panel().queryByText('Up next from recommendations')).toBeNull();
    // The window asks for one row more, to know what stands before it.
    const { offset } = bridge.queueRequests.at(-1) ?? { offset: -1 };
    expect(offset).toBeLessThan(50 - 4);
  });
});

describe('reordering the queue', () => {
  // Entry n has the key n + 9.
  it('drags an entry before another', async () => {
    const bridge = await open();
    placeList();

    await fireEvent.pointerDown(handle('Track 4'), at(3));
    await fireEvent.pointerMove(list(), at(1));
    await fireEvent.pointerUp(list(), at(1));

    expect(moves(bridge)).toEqual([['moveQueueEntry', 13, 11]]);
  });

  it('drags an entry to the end', async () => {
    const bridge = await open();
    placeList();

    await fireEvent.pointerDown(handle('Track 2'), at(1));
    await fireEvent.pointerMove(list(), at(9));
    await fireEvent.pointerUp(list(), at(9));

    expect(moves(bridge)).toEqual([['moveQueueEntry', 11, null]]);
  });

  it('sends nothing for a drop in place, Escape or a cancelled pointer', async () => {
    const bridge = await open();
    placeList();

    await fireEvent.pointerDown(handle('Track 2'), at(1));
    await fireEvent.pointerUp(list(), at(1));

    await fireEvent.pointerDown(handle('Track 2'), at(1));
    await fireEvent.pointerMove(list(), at(4));
    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.pointerUp(list(), at(4));

    await fireEvent.pointerDown(handle('Track 2'), at(1));
    await fireEvent.pointerMove(list(), at(4));
    await fireEvent.pointerCancel(list(), at(4));
    await fireEvent.pointerUp(list(), at(4));

    expect(moves(bridge)).toEqual([]);
  });

  it('keeps the dragged entry when the queue advances under it', async () => {
    const bridge = await open();
    placeList();
    await fireEvent.pointerDown(handle('Track 4'), at(3));

    // The first entry started playing: everything moved up one place.
    bridge.answerQueue = (offset, limit) => {
      const slice = queueSlice(offset + 1, limit, 5);
      return { ...slice, offset, total: 4, queueRevision: 2 };
    };
    bridge.emitPlayer(playerState({ revision: 2, upcoming: 4, queueRevision: 2 }));
    await waitFor(() => expect(items()).toHaveLength(4));
    await fireEvent.pointerMove(list(), at(0));
    await fireEvent.pointerUp(list(), at(0));

    expect(moves(bridge)).toEqual([['moveQueueEntry', 13, 11]]);
  });

  it('moves with the arrow keys, keeps the focus and says where it went', async () => {
    const bridge = await open();

    handle('Track 3').focus();
    await fireEvent.keyDown(handle('Track 3'), { key: 'ArrowUp' });
    expect(moves(bridge)).toEqual([['moveQueueEntry', 12, 11]]);
    expect(await panel().findByText('Moved Track 3 to position 2 of 5')).toBeTruthy();

    // Rust answers: Track 3 is now second.
    bridge.answerQueue = (offset, limit) => {
      const slice = queueSlice(offset, limit, 5, 2);
      [slice.items[1], slice.items[2]] = [slice.items[2], slice.items[1]];
      return slice;
    };
    bridge.emitPlayer(playerState({ revision: 2, upcoming: 5, queueRevision: 2 }));
    await waitFor(() => expect(document.activeElement?.getAttribute('data-key')).toBe('12'));
    await waitFor(() =>
      expect(items()[1].querySelector('[data-key]')?.getAttribute('data-key')).toBe('12'),
    );
    expect(document.activeElement).toBe(handle('Track 3'));

    await fireEvent.keyDown(handle('Track 3'), { key: 'ArrowDown' });
    // Back below Track 2: before the entry two places on.
    expect(moves(bridge)[1]).toEqual(['moveQueueEntry', 12, 13]);
  });

  it('ignores the arrows at the ends', async () => {
    const bridge = await open();

    await fireEvent.keyDown(handle('Track 1'), { key: 'ArrowUp' });
    await fireEvent.keyDown(handle('Track 5'), { key: 'ArrowDown' });

    expect(moves(bridge)).toEqual([]);
  });
});
