// The catalog views, driven through the whole app against the fake Rust side.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import { appStores, fakeBridge, library, searchView } from './lib/testing';

const sidebar = () => within(screen.getByRole('navigation', { name: 'Main' }));
const main = () => within(screen.getByRole('main'));
const mainText = () => screen.getByRole('main').textContent;
const pageTitle = () => main().getByRole('heading', { level: 1 }).textContent?.trim();
const section = (name: string) => within(main().getByRole('region', { name }));

async function openApp(bridge = fakeBridge()) {
  const view = render(App, {
    props: appStores(bridge),
  });
  await screen.findByRole('link', { name: 'Night shift, from YouTube Music' });
  return { ...view, bridge };
}

async function go(name: 'Home' | 'Search' | 'Library') {
  await fireEvent.click(sidebar().getByRole('link', { name }));
  await waitFor(() => expect(pageTitle()).toBe(name));
}

describe('Home', () => {
  const shelf = (name: string) => screen.findByRole('region', { name });

  it('shows a row of cards for every shelf Rust sends', async () => {
    await openApp();

    const first = within(await shelf('Jump back in'));

    expect(first.getAllByRole('link').map((link) => link.getAttribute('aria-label'))).toEqual([
      'Night shift, playlist from YouTube Music',
      'Low Tide, album from YouTube Music',
    ]);
    expect(section('From SoundCloud').getAllByRole('link')).toHaveLength(1);
  });

  it('puts the title, subtitle, badge and a local cover on a card', async () => {
    await openApp();

    const card = within(await shelf('Jump back in')).getByRole('link', { name: /low tide/i });

    for (const text of ['Low Tide', 'Mira Sol', 'YT', 'LT']) {
      expect(within(card).getByText(text)).toBeTruthy();
    }
    expect(card.querySelector('img')).toBeNull();
  });

  it('asks Rust to open a card and shows the album it reports', async () => {
    const { bridge } = await openApp();

    await fireEvent.click(
      within(await shelf('Jump back in')).getByRole('link', { name: /low tide/i }),
    );

    await waitFor(() => expect(pageTitle()).toBe('Low Tide'));
    expect(bridge.requested).toEqual([{ view: 'playlist', id: 'low-tide' }]);
    expect(mainText()).toContain('Album by Mira Sol');
    expect(mainText()).toContain('From YouTube');
  });

  it('offers scroll buttons only when a row has more cards than fit', async () => {
    await openApp();
    const region = within(await shelf('Jump back in'));
    const row = region.getByRole('list');
    expect(region.queryByRole('button')).toBeNull();

    // jsdom has no layout, so give the row a size.
    Object.defineProperty(row, 'clientWidth', { value: 400, configurable: true });
    Object.defineProperty(row, 'scrollWidth', { value: 900, configurable: true });
    const scrollBy = vi.fn();
    row.scrollBy = scrollBy;
    await fireEvent.scroll(row);

    const back = region.getByRole('button', { name: 'Scroll Jump back in back' });
    const forward = region.getByRole('button', { name: 'Scroll Jump back in forward' });
    expect(back.getAttribute('aria-disabled')).toBe('true');
    expect(forward.getAttribute('aria-disabled')).toBe('false');

    await fireEvent.click(back);
    expect(scrollBy).not.toHaveBeenCalled();
    await fireEvent.click(forward);
    expect(scrollBy).toHaveBeenCalledWith({ left: 400 });

    row.scrollLeft = 500;
    await fireEvent.scroll(row);
    expect(back.getAttribute('aria-disabled')).toBe('false');
    expect(forward.getAttribute('aria-disabled')).toBe('true');
    await fireEvent.click(back);
    expect(scrollBy).toHaveBeenLastCalledWith({ left: -400 });
  });

  it('says so when there is nothing to show', async () => {
    const bridge = fakeBridge();
    bridge.shelves = [];

    await openApp(bridge);

    expect(await screen.findByText('Nothing to play yet')).toBeTruthy();
  });

  it('says what went wrong when the shelves cannot be loaded', async () => {
    const bridge = fakeBridge();
    bridge.failCatalog = 'catalog unavailable';

    await openApp(bridge);

    expect((await screen.findByRole('alert')).textContent).toContain('catalog unavailable');
  });
});

describe('Search', () => {
  const box = () => main().getByRole<HTMLInputElement>('searchbox');
  const chip = (name: string) => main().getByRole<HTMLInputElement>('radio', { name });

  async function openSearch(bridge = fakeBridge()) {
    await openApp(bridge);
    await go('Search');
    await screen.findByRole('region', { name: 'Tracks' });
    return bridge;
  }

  it('starts with the search Rust remembers', async () => {
    const bridge = fakeBridge();
    bridge.remembered = { query: 'lamp', filter: 'youtube' };

    await openSearch(bridge);

    expect(box().value).toBe('lamp');
    expect(chip('YouTube').checked).toBe(true);
  });

  it('offers the All, YouTube and SoundCloud chips, with All chosen first', async () => {
    await openSearch();

    expect(
      main()
        .getAllByRole<HTMLInputElement>('radio')
        .map((radio) => [radio.labels?.[0]?.textContent?.trim(), radio.checked]),
    ).toEqual([
      ['All', true],
      ['YouTube', false],
      ['SoundCloud', false],
    ]);
  });

  it('sends every keystroke and chip change to Rust', async () => {
    const bridge = await openSearch();

    await fireEvent.input(box(), { target: { value: 'ha' } });
    await fireEvent.click(chip('SoundCloud'));
    await fireEvent.input(box(), { target: { value: 'har' } });
    await fireEvent.click(chip('All'));

    expect(bridge.searches).toEqual([
      { query: 'ha', filter: 'all' },
      { query: 'ha', filter: 'soundcloud' },
      { query: 'har', filter: 'soundcloud' },
      { query: 'har', filter: 'all' },
    ]);
    expect(box().value).toBe('har');
  });

  it('lists each track with its artists, album, length and every source badge', async () => {
    await openSearch();

    const [first, second] = section('Tracks').getAllByRole('listitem');

    for (const text of ['Harbor Lights', 'Mira Sol, Juno Park', 'Low Tide', '3:21', 'YT']) {
      expect(within(first).getByText(text)).toBeTruthy();
    }
    expect(within(second).getByText('SP')).toBeTruthy();
    expect(within(second).getByText('YT')).toBeTruthy();
  });

  it('shows collections beside the tracks and opens them through Rust', async () => {
    const bridge = await openSearch();

    await fireEvent.click(section('Albums and playlists').getByRole('link', { name: /low tide/i }));

    await waitFor(() => expect(pageTitle()).toBe('Low Tide'));
    expect(bridge.requested.at(-1)).toEqual({ view: 'playlist', id: 'low-tide' });
  });

  it('measures the card row again when a search brings other cards', async () => {
    const bridge = await openSearch();
    const region = section('Albums and playlists');
    const row = region.getByRole('list');
    Object.defineProperty(row, 'clientWidth', { value: 400, configurable: true });
    Object.defineProperty(row, 'scrollWidth', { value: 900, configurable: true });
    bridge.answer = (query, filter) => ({
      ...searchView(query, filter),
      collections: library().playlists,
    });

    await fireEvent.input(box(), { target: { value: 'n' } });

    expect(
      await region.findByRole('button', { name: 'Scroll Albums and playlists forward' }),
    ).toBeTruthy();
  });

  it('announces how much was found', async () => {
    await openSearch();

    expect(main().getByRole('status').textContent?.trim()).toBe('2 tracks, 1 album or playlist');
  });

  it('says what to try when Rust finds nothing', async () => {
    const bridge = fakeBridge();
    bridge.answer = (query, filter) => ({
      ...searchView(query, filter),
      tracks: [],
      collections: [],
    });
    await openApp(bridge);
    await go('Search');

    await fireEvent.input(box(), { target: { value: 'zzz' } });
    expect(await main().findByRole('heading', { name: 'No matches for “zzz”' })).toBeTruthy();
    expect(main().queryByRole('region')).toBeNull();

    await fireEvent.click(chip('SoundCloud'));
    await waitFor(() => expect(mainText()).toContain('choose All'));
  });

  it('comes back to the same search after another view', async () => {
    await openSearch();
    await fireEvent.input(box(), { target: { value: 'ferry' } });
    await fireEvent.click(chip('SoundCloud'));

    await go('Library');
    await go('Search');

    await waitFor(() => expect(box().value).toBe('ferry'));
    expect(chip('SoundCloud').checked).toBe(true);
  });

  it('does not reload the page when the form is submitted', async () => {
    await openSearch();

    const notCancelled = await fireEvent.submit(main().getByRole('search'));

    expect(notCancelled).toBe(false);
  });
});

describe('Library', () => {
  async function openLibrary() {
    const app = await openApp();
    await go('Library');
    await screen.findByRole('region', { name: 'Playlists' });
    return app;
  }

  it('lists the playlists and saved albums from Rust', async () => {
    await openLibrary();

    expect(section('Playlists').getAllByRole('link')).toHaveLength(3);
    expect(section('Saved albums').getAllByRole('link')).toHaveLength(1);
  });

  it('opens a playlist through Rust', async () => {
    const { bridge } = await openLibrary();

    await fireEvent.click(section('Playlists').getByRole('link', { name: /tunnel mixes/i }));

    await waitFor(() => expect(pageTitle()).toBe('Tunnel mixes'));
    expect(bridge.requested.at(-1)).toEqual({ view: 'playlist', id: 'tunnel-mixes' });
    expect(mainText()).toContain('Long, low and echoing');
    expect(mainText()).toContain('From SoundCloud');
  });

  it('says so when nothing is saved', async () => {
    const bridge = fakeBridge();
    bridge.saved = { playlists: [], albums: [] };
    await openApp(bridge);

    await go('Library');

    expect(await screen.findByText('Your library is empty')).toBeTruthy();
  });
});

describe('catalog views', () => {
  it('ask Rust once per visit and never poll', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { bridge } = await openApp();
      await screen.findByRole('region', { name: 'Jump back in' });

      await vi.advanceTimersByTimeAsync(60_000);

      expect(bridge.calls.filter((call) => call === 'getHome')).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
