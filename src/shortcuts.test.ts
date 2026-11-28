// The keyboard, through the whole app against the fake Rust side.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import { appStores, fakeBridge, playerState } from './lib/testing';

async function openApp(bridge = fakeBridge()) {
  bridge.accountStatus = 'connected';
  bridge.remembered = { query: '', filter: 'all', kind: 'all' };
  render(App, { props: appStores(bridge) });
  await screen.findByRole('link', { name: /^night shift, playlist$/i });
  await waitFor(() => expect(bridge.calls).toContain('getPlayerState'));
  return bridge;
}

const main = () => within(screen.getByRole('main'));
const searchbox = () => main().getByRole<HTMLInputElement>('searchbox');
const row = () =>
  main().findByRole('button', { name: 'Play Harbor Lights by Mira Sol, Juno Park' });

/** Whether the press was kept from the page. */
async function press(key: string, init: KeyboardEventInit = {}) {
  const target = document.activeElement ?? document.body;
  return !(await fireEvent.keyDown(target, { key, ...init }));
}

describe('keyboard shortcuts', () => {
  it('toggles play with Space', async () => {
    const bridge = await openApp();
    bridge.emitPlayer(playerState());

    expect(await press(' ')).toBe(true);
    expect(await press(' ', { repeat: true })).toBe(true);

    expect(bridge.commands).toEqual([['togglePlay']]);
  });

  it('pauses with Space after a row was clicked', async () => {
    const bridge = await openApp();
    await fireEvent.click(screen.getByRole('link', { name: 'Search' }));
    const play = await row();

    play.focus();
    await fireEvent.click(play, { detail: 1 });
    expect(document.activeElement).toBe(document.body);
    await press(' ');

    expect(bridge.commands).toEqual([['playSearchResult', 't1'], ['togglePlay']]);
  });

  it('keeps the focus on a row activated from the keyboard', async () => {
    const bridge = await openApp();
    await fireEvent.click(screen.getByRole('link', { name: 'Search' }));
    const play = await row();

    play.focus();
    await fireEvent.click(play, { detail: 0 });
    expect(document.activeElement).toBe(play);
    expect(await press(' ')).toBe(false);

    expect(bridge.commands).toEqual([['playSearchResult', 't1']]);
  });

  it('lets a space be typed in the search box', async () => {
    const bridge = await openApp();
    await fireEvent.click(screen.getByRole('link', { name: 'Search' }));
    await waitFor(() => expect(searchbox()).toBeTruthy());

    searchbox().focus();

    expect(await press(' ')).toBe(false);
    expect(bridge.commands).toEqual([]);
  });

  it('opens Search and focuses the box with Ctrl+F', async () => {
    const bridge = await openApp();

    expect(await press('f', { ctrlKey: true })).toBe(true);

    expect(bridge.requested).toEqual([{ view: 'search' }]);
    await waitFor(() => expect(document.activeElement).toBe(searchbox()));
  });

  it('focuses the box with Ctrl+F when already on Search', async () => {
    await openApp();
    await fireEvent.click(screen.getByRole('link', { name: 'Search' }));
    await waitFor(() => expect(searchbox()).toBeTruthy());
    expect(document.activeElement).not.toBe(searchbox());

    expect(await press('f', { ctrlKey: true })).toBe(true);

    await waitFor(() => expect(document.activeElement).toBe(searchbox()));
  });

  it('does not focus the box on a later, ordinary visit', async () => {
    await openApp();
    await press('f', { ctrlKey: true });
    await waitFor(() => expect(document.activeElement).toBe(searchbox()));

    await fireEvent.click(screen.getByRole('link', { name: 'Home' }));
    await fireEvent.click(screen.getByRole('link', { name: 'Search' }));
    await waitFor(() => expect(searchbox()).toBeTruthy());

    expect(document.activeElement).not.toBe(searchbox());
  });

  it('skips, seeks and changes the volume', async () => {
    const bridge = await openApp();

    await press('ArrowLeft', { ctrlKey: true });
    await press('ArrowRight', { ctrlKey: true });
    await press('ArrowRight', { shiftKey: true });
    await press('ArrowDown', { ctrlKey: true });

    expect(bridge.commands).toEqual([
      ['previousTrack'],
      ['nextTrack'],
      ['seekBy', 5000],
      ['changeVolume', -5],
    ]);
  });
});
