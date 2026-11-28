// Likes and history, driven through the whole app against the fake Rust side.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import { appStores, fakeBridge, playerState, tracks } from './lib/testing';

const sidebar = () => within(screen.getByRole('navigation', { name: 'Main' }));
const main = () => within(screen.getByRole('main'));
const bar = () => within(screen.getByRole('region', { name: 'Player' }));

function connected() {
  const bridge = fakeBridge();
  bridge.accountStatus = 'connected';
  return bridge;
}

async function openApp(bridge = connected()) {
  render(App, { props: appStores(bridge) });
  await screen.findByRole('link', { name: /^night shift, playlist$/i });
  await waitFor(() => expect(bridge.calls).toContain('getLibrarySummary'));
  return bridge;
}

async function go(name: 'Search' | 'Library') {
  await fireEvent.click(sidebar().getByRole('link', { name }));
  await waitFor(() => expect(screen.getByRole('heading', { level: 1 }).textContent).toBe(name));
}

const heart = (scope: ReturnType<typeof within>, title: string) =>
  scope.getByRole('button', { name: `Like ${title}` });
const pressed = (button: HTMLElement) => button.getAttribute('aria-pressed');

async function openResults(bridge = connected()) {
  await openApp(bridge);
  await go('Search');
  await screen.findByRole('region', { name: 'Songs' });
  return bridge;
}

describe('hearts', () => {
  it('shows a heart on each search result, pressed for the liked ones', async () => {
    const bridge = connected();
    bridge.likedIds = ['t2'];

    await openResults(bridge);

    await waitFor(() => expect(pressed(heart(main(), 'Lamplighter'))).toBe('true'));
    expect(pressed(heart(main(), 'Harbor Lights'))).toBe('false');
    expect(bridge.likedAsked).toEqual([['t1', 't2']]);
  });

  it('likes through Rust and shows it when the event arrives', async () => {
    const bridge = await openResults();

    await fireEvent.click(heart(main(), 'Harbor Lights'));

    expect(bridge.commands).toEqual([['likeTrack', 't1']]);
    await waitFor(() => expect(pressed(heart(main(), 'Harbor Lights'))).toBe('true'));
  });

  it('does not play the song when its heart is pressed', async () => {
    const bridge = await openResults();

    await fireEvent.click(heart(main(), 'Harbor Lights'));

    expect(bridge.commands.some(([name]) => name === 'playSearchResult')).toBe(false);
  });

  it('keeps the player bar and the results in step through the event', async () => {
    const bridge = await openResults();
    bridge.emitPlayer(playerState());
    await bar().findByText('Harbor Lights');

    await fireEvent.click(heart(bar(), 'Harbor Lights'));

    await waitFor(() => expect(pressed(heart(bar(), 'Harbor Lights'))).toBe('true'));
    expect(pressed(heart(main(), 'Harbor Lights'))).toBe('true');

    await fireEvent.click(heart(main(), 'Harbor Lights'));

    await waitFor(() => expect(pressed(heart(bar(), 'Harbor Lights'))).toBe('false'));
    expect(bridge.commands.map(([name]) => name)).toEqual(['likeTrack', 'unlikeTrack']);
  });

  it('has no heart for a local file', async () => {
    const bridge = await openApp();

    bridge.emitPlayer(
      playerState({ current: { key: 1, track: { ...tracks()[0], sources: [] }, autoplay: false } }),
    );
    await bar().findByText('Harbor Lights');

    expect(bar().queryByRole('button', { name: /^Like / })).toBeNull();
  });

  it('says why a like was refused', async () => {
    const bridge = await openResults();
    bridge.failCommands = 'the library was made by a newer Lumen';

    await fireEvent.click(heart(main(), 'Harbor Lights'));

    expect((await screen.findByRole('alert')).textContent).toContain('newer Lumen');
  });
});

describe('Library', () => {
  const part = (name: string) => main().getByRole<HTMLInputElement>('radio', { name });

  async function openLibrary(bridge = connected()) {
    await openApp(bridge);
    await go('Library');
    await screen.findByRole('region', { name: 'Playlists' });
    return bridge;
  }

  it('lists the liked songs as Rust sends them, newest like first', async () => {
    const bridge = connected();
    bridge.likedIds = ['t2', 't1'];
    bridge.rowsById.set('t1', tracks()[0]);
    bridge.rowsById.set('t2', tracks()[1]);
    await openLibrary(bridge);

    await fireEvent.click(part('Liked songs'));

    const table = await main().findByRole('table', { name: /Liked songs/ });
    await within(table).findByText('Lamplighter');
    const rows = within(table).getAllByRole('row').slice(1);
    expect(rows.map((row) => row.textContent)).toEqual([
      expect.stringContaining('Lamplighter'),
      expect.stringContaining('Harbor Lights'),
    ]);
    expect(bridge.calls).toContain('getLikedSlice');
  });

  it('plays a liked song alone by its id', async () => {
    const bridge = connected();
    bridge.likedIds = ['t1'];
    bridge.rowsById.set('t1', tracks()[0]);
    await openLibrary(bridge);
    await fireEvent.click(part('Liked songs'));

    await fireEvent.click(await main().findByRole('button', { name: /^Play Harbor Lights/ }));

    expect(bridge.commands).toEqual([['playLibraryTrack', 't1']]);
  });

  it('drops a song from the liked list when it is unliked', async () => {
    const bridge = connected();
    bridge.likedIds = ['t1'];
    bridge.rowsById.set('t1', tracks()[0]);
    await openLibrary(bridge);
    await fireEvent.click(part('Liked songs'));
    await within(await main().findByRole('table')).findByText('Harbor Lights');

    await fireEvent.click(heart(main(), 'Harbor Lights'));

    expect(await main().findByText('No liked songs yet')).toBeTruthy();
    expect(bridge.commands).toEqual([['unlikeTrack', 't1']]);
  });

  it('lists the songs played, the latest first, and says so when there are none', async () => {
    const bridge = connected();
    await openLibrary(bridge);
    await fireEvent.click(part('Recently played'));
    expect(await main().findByText('Nothing played yet')).toBeTruthy();

    bridge.played = [tracks()[1], tracks()[0]];
    bridge.emitHistory({ revision: 1, playedCount: 2 });

    const table = await main().findByRole('table', { name: /Recently played/ });
    await within(table).findByText('Lamplighter');
    expect(within(table).getAllByRole('row')).toHaveLength(3);
    expect(bridge.calls).toContain('getRecentSlice');
  });

  it('reports a library it could not open, and leaves the rest working', async () => {
    const bridge = connected();
    bridge.problem = 'the library was made by a newer Lumen';

    await openLibrary(bridge);

    expect((await main().findByRole('alert')).textContent).toContain('left the file as it was');
    expect(main().getByRole('region', { name: 'Playlists' })).toBeTruthy();
  });
});
