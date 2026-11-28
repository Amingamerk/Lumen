// The type chips and the shelves of albums, artists and playlists on the Search view,
// driven through the whole app against the fake Rust side.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import type { HitRow, SearchKind, SearchView } from './lib/ipc';
import { appStores, fakeBridge, searchView, tracks } from './lib/testing';

const sidebar = () => within(screen.getByRole('navigation', { name: 'Main' }));
const main = () => within(screen.getByRole('main'));
const section = (name: string) => within(main().getByRole('region', { name }));

const hit = (overrides: Partial<HitRow> & Pick<HitRow, 'id' | 'kind' | 'title'>): HitRow => ({
  by: [],
  detail: null,
  hasArt: true,
  ...overrides,
});

const album = hit({
  id: 'a1',
  kind: 'album',
  title: 'Night Market',
  by: ['Ada Brook'],
  detail: '2021',
});
const artist = hit({
  id: 'r1',
  kind: 'artist',
  title: 'Ada Brook',
  detail: '1.2M subscribers',
});
const playlist = hit({
  id: 'p9',
  kind: 'playlist',
  title: 'Ada Brook Essentials',
  by: ['Lumen Radio'],
  detail: '50 songs',
  hasArt: false,
});

/** What Rust sends for each chip: the top card and every shelf, or just the chip's own. */
function answer(query: string, filter: 'all' | 'youtube' | 'soundcloud', kind: SearchKind) {
  const songs = [...tracks(), { ...tracks()[0], id: 't3', title: 'Third Song' }];
  const view: Partial<SearchView> = { kind, tracks: [], albums: [], artists: [], playlists: [] };
  if (kind === 'all') {
    Object.assign(view, {
      top: { best: artist, songs: songs },
      tracks: songs,
      albums: [album],
      artists: [artist],
      playlists: [playlist],
    });
  }
  if (kind === 'songs') view.tracks = songs;
  if (kind === 'albums') view.albums = [album];
  if (kind === 'artists') view.artists = [artist];
  if (kind === 'playlists') view.playlists = [playlist];
  return searchView(query, filter, view);
}

async function openSearch() {
  const bridge = fakeBridge();
  bridge.accountStatus = 'connected';
  bridge.answer = answer;
  bridge.remembered = { query: 'ada', filter: 'all', kind: 'all' };
  render(App, { props: appStores(bridge) });
  await screen.findByRole('link', { name: 'Night shift, playlist' });
  await fireEvent.click(sidebar().getByRole('link', { name: 'Search' }));
  await screen.findByRole('region', { name: 'Top result' });
  return bridge;
}

const typeChip = (name: string) =>
  within(main().getByRole('group', { name: 'Type' })).getByRole<HTMLInputElement>('radio', {
    name,
  });

describe('Search types', () => {
  it('offers All, Songs, Albums, Artists and Playlists, with the remembered one chosen', async () => {
    await openSearch();

    const radios = within(main().getByRole('group', { name: 'Type' })).getAllByRole<HTMLInputElement>(
      'radio',
    );

    expect(radios.map((radio) => [radio.labels?.[0]?.textContent?.trim(), radio.checked])).toEqual([
      ['All', true],
      ['Songs', false],
      ['Albums', false],
      ['Artists', false],
      ['Playlists', false],
    ]);
  });

  it('shows the top result with three songs that play when clicked', async () => {
    const bridge = await openSearch();

    const top = section('Top result');
    expect(top.getByRole('heading', { name: 'Ada Brook' })).toBeTruthy();
    expect(top.getByText('Artist')).toBeTruthy();
    const plays = top.getAllByRole('button', { name: /^Play / });
    expect(plays).toHaveLength(3);

    await fireEvent.click(plays[0]);

    expect(bridge.commands).toEqual([['playSearchResult', tracks()[0].id]]);
  });

  it('shows albums, artists and playlists with a picture first and one line of detail', async () => {
    await openSearch();

    const albums = section('Albums');
    expect(albums.getByText('Night Market')).toBeTruthy();
    expect(albums.getByText('Album · Ada Brook · 2021')).toBeTruthy();
    expect(albums.getByRole('listitem').querySelector('img')?.getAttribute('src')).toBe(
      'http://lumen.localhost/art/64/a1',
    );
    expect(section('Artists').getByText('Artist · 1.2M subscribers')).toBeTruthy();
    const playlists = section('Playlists');
    expect(playlists.getByText('Playlist · Lumen Radio · 50 songs')).toBeTruthy();
    // No picture to ask for: the placeholder stays.
    expect(playlists.getByRole('listitem').querySelector('img')).toBeNull();
  });

  it('has no Shuffle, Mix or Radio buttons', async () => {
    await openSearch();

    expect(main().queryByRole('button', { name: /shuffle|mix|radio/i })).toBeNull();
  });

  it('lets a chip make Rust narrow the results, with no search sent from here', async () => {
    const bridge = await openSearch();

    await fireEvent.click(typeChip('Albums'));

    await waitFor(() => expect(bridge.kindChanges).toEqual(['albums']));
    await screen.findByRole('region', { name: 'Albums' });
    expect(main().queryByRole('region', { name: 'Songs' })).toBeNull();
    expect(main().queryByRole('region', { name: 'Top result' })).toBeNull();
    expect(main().queryByRole('region', { name: 'Artists' })).toBeNull();
    expect(typeChip('Albums').checked).toBe(true);
    expect(bridge.searches).toEqual([]);
  });

  it('shows only the songs on the Songs chip', async () => {
    await openSearch();

    await fireEvent.click(typeChip('Songs'));

    await screen.findByRole('region', { name: 'Songs' });
    expect(main().queryByRole('region', { name: 'Albums' })).toBeNull();
  });

  it('shows the loading state while a chip waits for Rust', async () => {
    const bridge = await openSearch();
    bridge.answer = (query, filter, kind) =>
      searchView(query, filter, { kind, status: 'loading', tracks: [] });

    await fireEvent.click(typeChip('Artists'));

    expect(await main().findByRole('heading', { name: 'Searching YouTube Music…' })).toBeTruthy();
  });

  it('keeps the type chips away from the SoundCloud page', async () => {
    const bridge = await openSearch();

    await fireEvent.click(
      within(main().getByRole('group', { name: 'Source' })).getByRole('radio', {
        name: 'SoundCloud',
      }),
    );

    await waitFor(() => expect(bridge.filterChanges).toEqual(['soundcloud']));
    expect(main().queryByRole('group', { name: 'Type' })).toBeNull();
  });
});
