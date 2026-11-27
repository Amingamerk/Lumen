// The playlist page, driven through the whole app against the fake Rust side.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import { appStores, fakeBridge } from './lib/testing';

/** Matches ROW_HEIGHT; the viewport fits ten rows. */
const ROW = 56;
const VIEWPORT = 10 * ROW;
/** Rows in view, one partly in view, and eight spare on each side. */
const MOST_ROWS = 10 + 1 + 2 * 8;

const main = () => within(screen.getByRole('main'));

function inMain(selector: string): HTMLElement {
  const found = screen.getByRole('main').querySelector<HTMLElement>(selector);
  if (!found) throw new Error(`nothing matches ${selector}`);
  return found;
}

async function openPlaylist(count: number, bridge = fakeBridge()) {
  bridge.trackCounts['night-shift'] = count;
  const view = render(App, {
    props: appStores(bridge),
  });
  await fireEvent.click(
    await screen.findByRole('link', { name: /^night shift, from/i }),
  );
  await main().findByRole('heading', { level: 1, name: 'Night shift' });
  return { ...view, bridge };
}

/** The data rows in the DOM, without the column headings. */
const rows = () =>
  main()
    .queryAllByRole('row')
    .filter((row) => row.getAttribute('aria-rowindex') !== '1');
const row = (position: number) =>
  rows().find((each) => each.getAttribute('aria-rowindex') === String(position + 1));
function inRow(position: number) {
  const found = row(position);
  if (!found) throw new Error(`row ${position} is not drawn`);
  return within(found);
}

/** jsdom has no layout, so give the scroller a size and scroll it by hand. */
async function scrollTo(scrolled: number) {
  const scroller = inMain('[data-scroller]');
  const body = main().getAllByRole('rowgroup')[1];
  Object.defineProperty(scroller, 'clientHeight', { value: VIEWPORT, configurable: true });
  body.getBoundingClientRect = () => new DOMRect(0, -scrolled);
  await fireEvent.scroll(scroller);
}

describe('playlist page', () => {
  it('heads the page with a cover, the source, the track count and the length', async () => {
    await openPlaylist(5000);

    const text = inMain('header').textContent?.replace(/\s+/g, ' ');
    expect(text).toContain('Slow songs for after midnight');
    expect(text).toContain('From YouTube Music');
    expect(text).toContain('5,000 tracks');
    expect(text).toContain('279 hr 10 min');
  });

  it('lights the backdrop with the colour of the cover', async () => {
    await openPlaylist(2);

    const tone = inMain('header .art').getAttribute('data-tone');
    expect(tone).toMatch(/^[0-5]$/);
    expect(inMain('header').getAttribute('data-tone')).toBe(tone);
  });

  it('lists index, title, artists, album, length and source badges', async () => {
    await openPlaylist(2);
    await waitFor(() => expect(rows()).toHaveLength(2));

    expect(
      main()
        .getAllByRole('columnheader')
        .map((cell) => cell.textContent?.trim()),
    ).toEqual(['#', 'Title', 'Album', 'Source', 'Time']);
    for (const text of ['2', 'Track 2', 'Mira Sol, Juno Park', 'Low Tide', '3:21', 'YT']) {
      expect(inRow(2).getByText(text)).toBeTruthy();
    }
    expect(main().getByRole('table', { name: 'Tracks' }).getAttribute('aria-rowcount')).toBe('3');
  });

  it('asks Rust for the first page only and draws only the rows near the view', async () => {
    const { bridge } = await openPlaylist(5000);
    await waitFor(() => expect(row(1)).toBeTruthy());

    await scrollTo(0);

    expect(bridge.slices).toEqual([{ id: 'night-shift', offset: 0, limit: 50 }]);
    expect(rows()).toHaveLength(18);
    expect(main().getByRole('table').getAttribute('aria-rowcount')).toBe('5001');
  });

  it('keeps the rows bounded at the middle and at track 5,000', async () => {
    const { bridge } = await openPlaylist(5000);
    await waitFor(() => expect(row(1)).toBeTruthy());

    await scrollTo(2500 * ROW);
    await waitFor(() => expect(main().getByText('Track 2501')).toBeTruthy());
    expect(rows().length).toBeLessThanOrEqual(MOST_ROWS);
    expect(row(1)).toBeUndefined();

    await scrollTo(5000 * ROW - VIEWPORT);
    await waitFor(() => expect(main().getByText('Track 5000')).toBeTruthy());
    expect(rows().length).toBeLessThanOrEqual(MOST_ROWS);
    expect(inRow(5000).getByText('5,000')).toBeTruthy();

    await scrollTo(0);
    await waitFor(() => expect(main().getByText('Track 1')).toBeTruthy());
    expect(bridge.slices.map((slice) => slice.offset)).toEqual([0, 2450, 2500, 4950]);
  });

  it('does not ask again while scrolling within what it has', async () => {
    const { bridge } = await openPlaylist(5000);
    await waitFor(() => expect(row(1)).toBeTruthy());

    for (let scrolled = 0; scrolled <= 20 * ROW; scrolled += 40) await scrollTo(scrolled);

    expect(bridge.slices).toHaveLength(1);
  });

  it('holds the place of a row that is still on its way', async () => {
    const { bridge } = await openPlaylist(5000);
    await waitFor(() => expect(row(1)).toBeTruthy());
    bridge.beforeSlice = () => new Promise(() => {});

    await scrollTo(1000 * ROW);

    expect(inRow(1001).getByText('1,001')).toBeTruthy();
    expect(inRow(1001).getByText('Loading…')).toBeTruthy();
  });

  it('says so when a playlist is empty and asks for no rows', async () => {
    const { bridge } = await openPlaylist(0);

    expect(main().getByRole('heading', { name: 'No tracks here yet' })).toBeTruthy();
    expect(main().queryByRole('table')).toBeNull();
    expect(bridge.slices).toEqual([]);
  });

  it('says what went wrong when the rows cannot be loaded', async () => {
    const bridge = fakeBridge();
    bridge.failSlices = 'tracks unavailable';

    await openPlaylist(5000, bridge);

    expect((await screen.findByRole('alert')).textContent).toContain('tracks unavailable');
  });

  it('opens each playlist at its own top, with its own rows', async () => {
    const { bridge } = await openPlaylist(5000);
    await waitFor(() => expect(row(1)).toBeTruthy());

    await fireEvent.click(screen.getByRole('link', { name: 'Tunnel mixes, from SoundCloud' }));

    await waitFor(() => expect(bridge.slices.at(-1)?.id).toBe('tunnel-mixes'));
    expect(bridge.slices.at(-1)?.offset).toBe(0);
    await waitFor(() => expect(rows()).toHaveLength(2));
  });
});
