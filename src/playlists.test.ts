// Making, filling, renaming and removing playlists, through the whole app.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import { appStores, fakeBridge } from './lib/testing';

const sidebar = () => within(screen.getByRole('navigation', { name: 'Main' }));
const main = () => within(screen.getByRole('main'));
const pageTitle = () => main().getByRole('heading', { level: 1 }).textContent?.trim();

async function openApp(bridge = fakeBridge()) {
  render(App, { props: appStores(bridge) });
  await sidebar().findByRole('link', { name: /^night shift, playlist$/i });
  return bridge;
}

async function openNightShift(bridge = fakeBridge()) {
  await openApp(bridge);
  await fireEvent.click(sidebar().getByRole('link', { name: /^night shift, playlist$/i }));
  await waitFor(() => expect(pageTitle()).toBe('Night shift'));
  return bridge;
}

describe('making a playlist', () => {
  it('names it in the sidebar, then opens it', async () => {
    const bridge = await openApp();

    await fireEvent.click(sidebar().getByRole('button', { name: 'New playlist' }));
    const box = sidebar().getByRole<HTMLInputElement>('textbox', {
      name: 'Name of the new playlist',
    });
    await fireEvent.input(box, { target: { value: 'Tunnel mixes' } });
    await fireEvent.submit(box.closest('form') as HTMLFormElement);

    await waitFor(() => expect(pageTitle()).toBe('Tunnel mixes'));
    expect(bridge.commands).toContainEqual(['createPlaylist', 'Tunnel mixes']);
    expect(bridge.requested.at(-1)).toEqual({ view: 'playlist', id: 'p2' });
    expect(sidebar().getByRole('link', { name: 'Tunnel mixes, playlist' })).toBeTruthy();
    expect(main().getByRole('heading', { name: 'No songs here yet' })).toBeTruthy();
  });

  it('makes nothing from a blank name', async () => {
    const bridge = await openApp();

    await fireEvent.click(sidebar().getByRole('button', { name: 'New playlist' }));
    const box = sidebar().getByRole('textbox', { name: 'Name of the new playlist' });
    await fireEvent.submit(box.closest('form') as HTMLFormElement);

    expect(bridge.commands).toEqual([]);
  });
});

describe('adding songs', () => {
  async function openResults(bridge = fakeBridge()) {
    bridge.accountStatus = 'connected';
    bridge.remembered = { query: '', filter: 'all', kind: 'all' };
    await openApp(bridge);
    await fireEvent.click(sidebar().getByRole('link', { name: 'Search' }));
    await main().findByRole('region', { name: 'Songs' });
    return bridge;
  }

  const picker = () => within(screen.getByRole('dialog'));

  it('puts a search result in the chosen playlist', async () => {
    const bridge = await openResults();

    await fireEvent.click(main().getByRole('button', { name: /^Add Harbor Lights to a playlist$/ }));
    await fireEvent.click(await picker().findByRole('button', { name: /^Night shift/ }));

    expect(bridge.commands).toEqual([['addToPlaylist', 'night-shift', 't1']]);
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(bridge.lists[0].entries).toHaveLength(3);
    expect(
      await sidebar().findByRole('link', { name: /^night shift, playlist$/i }),
    ).toBeTruthy();
  });

  it('adds the same song again as a second entry', async () => {
    const bridge = await openResults();
    for (let again = 0; again < 2; again += 1) {
      await fireEvent.click(
        main().getByRole('button', { name: /^Add Harbor Lights to a playlist$/ }),
      );
      await fireEvent.click(await picker().findByRole('button', { name: /^Night shift/ }));
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    }

    expect(bridge.lists[0].entries.map((entry) => entry.id)).toEqual(['n1', 'n2', 't1', 't1']);
  });

  it('makes a playlist for the song and puts it in', async () => {
    const bridge = await openResults();

    await fireEvent.click(main().getByRole('button', { name: /^Add Lamplighter to a playlist$/ }));
    const box = picker().getByRole('textbox', { name: 'Name of a new playlist' });
    await fireEvent.input(box, { target: { value: 'Fresh' } });
    await fireEvent.submit(box.closest('form') as HTMLFormElement);

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(bridge.commands).toEqual([
      ['createPlaylist', 'Fresh'],
      ['addToPlaylist', 'p2', 't2'],
    ]);
  });

  it('closes with Escape and adds nothing', async () => {
    const bridge = await openResults();
    await fireEvent.click(main().getByRole('button', { name: /^Add Harbor Lights to a playlist$/ }));

    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(bridge.commands).toEqual([]);
  });

  it('does not play the song when its add button is pressed', async () => {
    const bridge = await openResults();

    await fireEvent.click(main().getByRole('button', { name: /^Add Harbor Lights to a playlist$/ }));

    expect(bridge.commands).toEqual([]);
  });
});

describe('a playlist page', () => {
  it('plays the song of a row by its id', async () => {
    const bridge = await openNightShift();

    await fireEvent.click(await main().findByRole('button', { name: /^Play Track 2 by/ }));

    expect(bridge.commands).toEqual([['playLibraryTrack', 'n2']]);
  });

  it('removes the entry of a row and shows the new count', async () => {
    const bridge = await openNightShift();

    await fireEvent.click(
      await main().findByRole('button', { name: 'Remove Track 1 from this playlist' }),
    );

    expect(bridge.commands).toEqual([['removeFromPlaylist', 'night-shift', 0, 'n1']]);
    await waitFor(() => expect(screen.getByRole('main').textContent).toContain('1 track'));
    await waitFor(() => expect(main().queryByText('Track 1')).toBeNull());
    expect(main().getByText('Track 2')).toBeTruthy();
  });

  it('renames the playlist', async () => {
    const bridge = await openNightShift();

    await fireEvent.click(main().getByRole('button', { name: 'Rename' }));
    const box = main().getByRole<HTMLInputElement>('textbox', { name: 'Playlist name' });
    await fireEvent.input(box, { target: { value: 'Late shift' } });
    await fireEvent.submit(box.closest('form') as HTMLFormElement);

    await waitFor(() => expect(pageTitle()).toBe('Late shift'));
    expect(bridge.commands).toEqual([['renamePlaylist', 'night-shift', 'Late shift']]);
    expect(sidebar().getByRole('link', { name: 'Late shift, playlist' })).toBeTruthy();
  });

  it('asks before deleting, then returns to the Library', async () => {
    const bridge = await openNightShift();

    await fireEvent.click(main().getByRole('button', { name: 'Delete playlist' }));
    expect(bridge.commands).toEqual([]);
    await fireEvent.click(main().getByRole('button', { name: 'Keep it' }));
    expect(bridge.commands).toEqual([]);
    await fireEvent.click(main().getByRole('button', { name: 'Delete playlist' }));
    await fireEvent.click(main().getByRole('button', { name: /^Delete the playlist/ }));

    await waitFor(() => expect(pageTitle()).toBe('Library'));
    expect(bridge.commands).toEqual([['deletePlaylist', 'night-shift']]);
    expect(await sidebar().findByText('No playlists yet')).toBeTruthy();
  });

  it('says so when its playlist is gone', async () => {
    const bridge = await openNightShift();

    bridge.lists = [];
    bridge.emitPlaylists({ revision: 30, playlists: [], changed: 'night-shift' });

    expect(await main().findByRole('heading', { name: 'This playlist is gone' })).toBeTruthy();
  });
});
