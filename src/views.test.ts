// The catalog views, driven through the whole app against the fake Rust side.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import App from './App.svelte';
import type { SearchStatus, SourceFilter } from './lib/ipc';
import { appStores, fakeBridge, searchView, serviceStatus, tracks } from './lib/testing';

const sidebar = () => within(screen.getByRole('navigation', { name: 'Main' }));
const main = () => within(screen.getByRole('main'));
const mainText = () => screen.getByRole('main').textContent;
const pageTitle = () => main().getByRole('heading', { level: 1 }).textContent?.trim();
const section = (name: string) => within(main().getByRole('region', { name }));

async function openApp(bridge = fakeBridge()) {
  const view = render(App, {
    props: appStores(bridge),
  });
  await screen.findByRole('link', { name: 'Night shift, playlist' });
  return { ...view, bridge };
}

async function go(name: 'Home' | 'Search' | 'Library') {
  await fireEvent.click(sidebar().getByRole('link', { name }));
  await waitFor(() => expect(pageTitle()).toBe(name));
}

describe('Home', () => {
  const shelf = (name: string) => screen.findByRole('region', { name });

  /** A listener who has played two songs, liked one and made a playlist. */
  function listener() {
    const bridge = fakeBridge();
    const [first, second] = tracks();
    bridge.played = [second, first];
    bridge.likedIds = [first.id];
    bridge.rowsById.set(first.id, first);
    return bridge;
  }

  it('shows the recent songs, likes and playlists of the library', async () => {
    await openApp(listener());

    const recent = within(await shelf('Recently played'));
    expect(recent.getAllByRole('button', { name: /^Play/ }).map((b) => b.getAttribute('aria-label'))).toEqual([
      'Play Lamplighter, Juno Park',
      'Play Harbor Lights, Mira Sol, Juno Park',
    ]);
    expect(section('Liked songs').getAllByRole('button', { name: /^Play/ })).toHaveLength(1);
    expect(section('Your playlists').getAllByRole('link')).toHaveLength(1);
  });

  it('puts a lazy local cover on a card', async () => {
    await openApp(listener());

    const card = within(await shelf('Recently played')).getByRole('button', { name: /harbor/i });

    expect(card.querySelector('img')?.getAttribute('loading')).toBe('lazy');
    expect(card.querySelector('img')?.getAttribute('src')).toContain('art/300/t1');
  });

  it('plays a song card through the queue in the shelf order', async () => {
    const bridge = listener();
    await openApp(bridge);

    await fireEvent.click(
      within(await shelf('Recently played')).getByRole('button', { name: /harbor/i }),
    );

    expect(bridge.commands).toContainEqual(['playList', { kind: 'recent' }, 1, 't1']);
  });

  it('asks Rust to open a playlist card and shows the playlist it reports', async () => {
    const bridge = listener();
    await openApp(bridge);

    await fireEvent.click(
      within(await shelf('Your playlists')).getByRole('link', { name: /night shift/i }),
    );

    await waitFor(() => expect(pageTitle()).toBe('Night shift'));
    expect(bridge.requested).toEqual([{ view: 'playlist', id: 'night-shift' }]);
  });

  it('links Show all to the matching Library tab', async () => {
    await openApp(listener());

    await fireEvent.click(within(await shelf('Liked songs')).getByRole('button', { name: /show all/i }));

    await waitFor(() => expect(pageTitle()).toBe('Library'));
    expect(main().getByRole<HTMLInputElement>('radio', { name: 'Liked songs' }).checked).toBe(true);
  });

  it('offers scroll buttons only when a row has more cards than fit', async () => {
    await openApp();
    const region = within(await shelf('Your playlists'));
    const row = region.getByRole('list');
    expect(region.queryByRole('button', { name: /^Scroll/ })).toBeNull();

    // jsdom has no layout, so give the row a size.
    Object.defineProperty(row, 'clientWidth', { value: 400, configurable: true });
    Object.defineProperty(row, 'scrollWidth', { value: 900, configurable: true });
    const scrollBy = vi.fn();
    row.scrollBy = scrollBy;
    await fireEvent.scroll(row);

    const back = region.getByRole('button', { name: 'Scroll Your playlists back' });
    const forward = region.getByRole('button', { name: 'Scroll Your playlists forward' });
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
    bridge.lists = [];

    render(App, { props: appStores(bridge) });

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
  const enter = () => fireEvent.submit(main().getByRole('search'));
  const type = (value: string) => fireEvent.input(box(), { target: { value } });
  const heading = (name: string | RegExp) => main().findByRole('heading', { name });

  /** A signed-in bridge. */
  function connected() {
    const bridge = fakeBridge();
    bridge.accountStatus = 'connected';
    return bridge;
  }

  /** A bridge whose search stands at `status`, with nothing found. */
  function standing(status: SearchStatus, filter: SourceFilter = 'all') {
    const bridge = connected();
    bridge.remembered = { query: 'zzz', filter, kind: 'all' };
    bridge.answer = (query, chosen) =>
      searchView(query, chosen, { status, tracks: [], collections: [] });
    return bridge;
  }

  async function openSearch(bridge = connected()) {
    await openApp(bridge);
    await go('Search');
    await waitFor(() => expect(bridge.calls).toContain('getSearch'));
    return bridge;
  }

  async function openResults(bridge = connected()) {
    await openSearch(bridge);
    await screen.findByRole('region', { name: /^(Songs|Tracks)$/ });
    return bridge;
  }

  it('starts with the search Rust remembers', async () => {
    const bridge = connected();
    bridge.remembered = { query: 'lamp', filter: 'youtube', kind: 'all' };

    await openResults(bridge);

    expect(box().value).toBe('lamp');
    expect(chip('YouTube').checked).toBe(true);
  });

  it('offers the All, YouTube and SoundCloud chips, with All chosen first', async () => {
    await openResults();

    expect(
      within(main().getByRole('group', { name: 'Source' }))
        .getAllByRole<HTMLInputElement>('radio')
        .map((radio) => [radio.labels?.[0]?.textContent?.trim(), radio.checked]),
    ).toEqual([
      ['All', true],
      ['YouTube', false],
      ['SoundCloud', false],
    ]);
  });

  it('submits on Enter only, never while typing or choosing a chip', async () => {
    const bridge = await openResults();

    await type('ha');
    await fireEvent.click(chip('SoundCloud'));
    await type('har');
    await fireEvent.click(chip('YouTube'));

    expect(bridge.searches).toEqual([]);
    expect(bridge.filterChanges).toEqual(['soundcloud', 'youtube']);
    expect(box().value).toBe('har');

    const notCancelled = await enter();

    expect(notCancelled).toBe(false);
    expect(bridge.searches).toEqual([{ query: 'har', filter: 'youtube' }]);
  });

  it('keeps what is being typed when a result arrives', async () => {
    const bridge = await openResults();
    await type('half typ');

    bridge.emitSearch(searchView('older', 'all', { revision: 9 }));

    expect(box().value).toBe('half typ');
  });

  it('lists each song with its artists, album, length and source badge', async () => {
    await openResults();

    const [first, second] = section('Songs').getAllByRole('listitem');

    for (const text of ['Harbor Lights', 'Mira Sol, Juno Park · Low Tide · 3:21', 'YT']) {
      expect(within(first).getByText(text)).toBeTruthy();
    }
    expect(within(second).getByText('YT')).toBeTruthy();
    expect(main().getByRole('status').textContent?.trim()).toBe('2 songs');
  });

  it('asks Rust to play the live song that was chosen', async () => {
    const bridge = await openResults();

    await fireEvent.click(
      main().getByRole('button', { name: 'Play Harbor Lights by Mira Sol, Juno Park' }),
    );

    expect(bridge.commands).toEqual([['playSearchResult', expect.any(String)]]);
  });

  it('says SoundCloud arrives later on its chip and sends nothing to a service', async () => {
    const bridge = connected();
    bridge.remembered = { query: '', filter: 'soundcloud', kind: 'all' };
    await openSearch(bridge);

    expect(await heading('SoundCloud arrives later')).toBeTruthy();
    expect(main().queryByRole('region', { name: 'Songs' })).toBeNull();
    expect(main().queryByRole('button', { name: 'Sign in' })).toBeNull();
    expect(bridge.searches).toEqual([]);
  });

  it('says it again after the chip is chosen, and never lists sample results', async () => {
    await openResults();

    await fireEvent.click(chip('SoundCloud'));

    expect(await heading('SoundCloud arrives later')).toBeTruthy();
    expect(main().queryByRole('region', { name: 'Songs' })).toBeNull();
  });

  it('asks for Enter before anything was searched', async () => {
    await openSearch(standing('idle'));

    expect(await heading('Press Enter to search')).toBeTruthy();
  });

  it('says it is searching', async () => {
    await openSearch(standing('loading'));

    expect(await heading('Searching YouTube Music…')).toBeTruthy();
  });

  it('says what to try when nothing is found', async () => {
    await openSearch(standing('noResults'));

    expect(await heading('No matches for “zzz”')).toBeTruthy();
    expect(main().queryByRole('region')).toBeNull();
  });

  it.each([
    ['offline', 'Could not reach YouTube Music', 'internet connection'],
    ['unexpected', 'YouTube Music gave an unexpected answer', 'Three in a row pause'],
  ] as const)('explains %s and submits once on Try again', async (status, title, advice) => {
    const bridge = await openSearch(standing(status));
    expect(await heading(title)).toBeTruthy();
    expect(mainText()).toContain(advice);

    await fireEvent.click(main().getByRole('button', { name: 'Try again' }));

    expect(bridge.searches).toEqual([{ query: 'zzz', filter: 'all' }]);
  });

  it.each([
    ['disconnected', 'Sign in to search YouTube Music', 'Sign in'],
    ['expired', 'Your session expired', 'Log in again'],
  ] as const)('shows %s with its way back in', async (account, title, action) => {
    const bridge = fakeBridge();
    bridge.accountStatus = account;
    await openSearch(bridge);
    expect(await heading(title)).toBeTruthy();
    expect(main().queryByRole('region')).toBeNull();

    await fireEvent.click(main().getByRole('button', { name: action }));

    expect(bridge.commands).toEqual([['signIn']]);
    expect(bridge.searches).toEqual([]);
  });

  it('shows a cool-down in place of the results, with no button', async () => {
    const bridge = connected();
    bridge.youtubeStatus = serviceStatus({
      gate: { state: 'coolingDown', untilMs: Date.now() + 120_000 },
    });

    await openSearch(bridge);

    await waitFor(() => expect(mainText()).toContain('asked Lumen to slow down'));
    expect(mainText()).toContain('until');
    expect(main().queryByRole('region')).toBeNull();
    expect(main().queryByRole('button', { name: /resume|try again/i })).toBeNull();
  });

  it('shows a pause with Resume, which sends no search', async () => {
    const bridge = connected();
    bridge.youtubeStatus = serviceStatus({ gate: { state: 'paused', reason: 'botCheck' } });
    await openSearch(bridge);

    await fireEvent.click(await main().findByRole('button', { name: 'Resume' }));

    expect(mainText()).toContain('Check your account in a browser first');
    expect(main().queryByRole('region')).toBeNull();
    expect(bridge.commands).toEqual([['resumeYoutube']]);
    expect(bridge.searches).toEqual([]);
  });

  it('reads blocked as waiting for Enter once nothing blocks', async () => {
    await openSearch(standing('blocked'));

    expect(await heading('Press Enter to search')).toBeTruthy();
  });

  it('comes back to the same search after another view', async () => {
    await openResults();
    await type('ferry');
    await fireEvent.click(chip('SoundCloud'));
    await enter();

    await go('Library');
    await go('Search');

    await waitFor(() => expect(box().value).toBe('ferry'));
    expect(chip('SoundCloud').checked).toBe(true);
  });
});

describe('Library', () => {
  async function openLibrary(bridge = fakeBridge()) {
    const app = await openApp(bridge);
    await go('Library');
    return app;
  }

  it('lists the local playlists from Rust', async () => {
    const bridge = fakeBridge();
    bridge.lists.push({ id: 'p2', name: 'Tunnel mixes', entries: [] });
    await openLibrary(bridge);

    const region = await screen.findByRole('region', { name: 'Playlists' });

    expect(within(region).getAllByRole('link')).toHaveLength(2);
  });

  it('opens a playlist through Rust', async () => {
    const bridge = fakeBridge();
    bridge.lists.push({ id: 'p2', name: 'Tunnel mixes', entries: [] });
    await openLibrary(bridge);

    await fireEvent.click(await section('Playlists').findByRole('link', { name: /tunnel mixes/i }));

    await waitFor(() => expect(pageTitle()).toBe('Tunnel mixes'));
    expect(bridge.requested.at(-1)).toEqual({ view: 'playlist', id: 'p2' });
  });

  it('says so when there are no playlists', async () => {
    const bridge = fakeBridge();
    bridge.lists = [];
    render(App, { props: appStores(bridge) });
    await sidebar().findByText('No playlists yet');

    await go('Library');

    expect(await screen.findByText('Your library is empty')).toBeTruthy();
  });
});

describe('catalog views', () => {
  it('ask Rust once per visit and never poll', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { bridge } = await openApp();
      await screen.findByRole('region', { name: 'Your playlists' });

      await vi.advanceTimersByTimeAsync(60_000);

      expect(bridge.calls.filter((call) => call === 'getHome')).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
