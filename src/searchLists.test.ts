// Every list on the Search view draws only the rows near the view, and a song is one
// thumbnail-first row, driven through the whole app against the fake Rust side.
import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import type { HitRow, TrackRow } from './lib/ipc';
import { appStores, fakeBridge, searchView, tracks } from './lib/testing';

const main = () => within(screen.getByRole('main'));
const section = (name: string) => within(main().getByRole('region', { name }));
const sidebar = () => within(screen.getByRole('navigation', { name: 'Main' }));

const ROW = 64;
const VIEWPORT = 10 * ROW;
const MOST_ROWS = 10 + 2 * 8 + 1;

const songs = (count: number): TrackRow[] =>
  Array.from({ length: count }, (_, at) => ({
    ...tracks()[0],
    id: `s${at}`,
    title: `Song ${at + 1}`,
  }));
const hits = (kind: HitRow['kind'], count: number): HitRow[] =>
  Array.from({ length: count }, (_, at) => ({
    id: `${kind}${at}`,
    kind,
    title: `${kind} ${at + 1}`,
    by: [],
    detail: null,
    hasArt: false,
  }));

async function openSearch(count = 500) {
  const bridge = fakeBridge();
  bridge.accountStatus = 'connected';
  bridge.answer = (query, filter) =>
    searchView(query, filter, {
      tracks: songs(count),
      albums: hits('album', count),
      artists: hits('artist', count),
      playlists: hits('playlist', count),
    });
  bridge.remembered = { query: 'ada', filter: 'all', kind: 'all' };
  render(App, { props: appStores(bridge) });
  await screen.findByRole('link', { name: 'Night shift, playlist' });
  await fireEvent.click(sidebar().getByRole('link', { name: 'Search' }));
  await screen.findByRole('region', { name: 'Songs' });
  return bridge;
}

/** jsdom has no layout, so give the scroller a size and scroll the list by hand. */
async function scrollList(name: string, scrolled: number) {
  const scroller = screen.getByRole('main').querySelector<HTMLElement>('[data-scroller]') as HTMLElement;
  const list = within(main().getByRole('region', { name })).getByRole('list');
  Object.defineProperty(scroller, 'clientHeight', { value: VIEWPORT, configurable: true });
  list.getBoundingClientRect = () => new DOMRect(0, -scrolled);
  await fireEvent.scroll(scroller);
}

describe('Search lists', () => {
  it.each(['Songs', 'Albums', 'Artists', 'Playlists'])(
    'draws a bounded number of %s rows from a long list',
    async (name) => {
      await openSearch();

      expect(section(name).getAllByRole('listitem').length).toBeLessThanOrEqual(MOST_ROWS);

      await scrollList(name, 300 * ROW);
      const rows = section(name).getAllByRole('listitem');
      expect(rows.length).toBeLessThanOrEqual(MOST_ROWS);
      expect(rows.length).toBeGreaterThan(0);
      expect(section(name).getByRole('list').getAttribute('style')).toContain(`${500 * ROW}px`);
    },
  );

  it('shows a song as thumbnail, title, then artists, album and length on one line', async () => {
    await openSearch(2);

    const [first] = section('Songs').getAllByRole('listitem');
    const text = (selector: string) => first.querySelector(selector)?.textContent?.trim();
    expect(first.querySelector('.art')).toBeTruthy();
    expect(text('.track-title')).toBe('Song 1');
    expect(text('.track-meta')).toBe('Mira Sol, Juno Park · Low Tide · 3:21');
  });

  it('keeps play, like and add-to-playlist on a song row', async () => {
    const bridge = await openSearch(2);
    const songsList = section('Songs');

    await fireEvent.click(songsList.getAllByRole('button', { name: /^Like Song 1/ })[0]);
    await fireEvent.click(songsList.getByRole('button', { name: 'Add Song 1 to a playlist' }));
    await fireEvent.click(
      songsList.getByRole('button', { name: 'Play Song 1 by Mira Sol, Juno Park' }),
    );

    expect(bridge.commands.map(([name]) => name)).toContain('playSearchResult');
    expect(bridge.commands.map(([name]) => name)).toContain('likeTrack');
  });
});
