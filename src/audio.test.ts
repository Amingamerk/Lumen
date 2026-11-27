// Real audio as the app shows it: the new statuses, the notice, and no Queue.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import type { PlayerState } from './lib/ipc';
import { appStores, fakeBridge, playerState } from './lib/testing';

async function openApp(bridge = fakeBridge()) {
  render(App, { props: appStores(bridge) });
  await screen.findByRole('link', { name: /^night shift, from/i });
  await waitFor(() => expect(bridge.calls).toContain('getPlayerState'));
  return bridge;
}

const bar = () => within(screen.getByRole('region', { name: 'Player' }));
const button = (name: string) => bar().getByRole<HTMLButtonElement>('button', { name });
const slider = (name: string) => bar().getByRole<HTMLInputElement>('slider', { name });
const notice = () => screen.queryByRole('alert');

/** What Rust sends for a file given with `--play-file`. */
function fileState(overrides: Partial<PlayerState> = {}): PlayerState {
  return playerState({
    current: {
      key: 1,
      autoplay: false,
      track: {
        id: '00000000-0000-0000-0000-000000000000',
        title: 'tone',
        artists: [],
        album: null,
        durationMs: 10_000,
        sources: [],
      },
    },
    durationMs: 10_000,
    canNext: false,
    canPrevious: false,
    upcoming: 0,
    ...overrides,
  });
}

const FAILED = 'Lumen could not find “tone”.';
const failed = (revision = 2) =>
  fileState({ revision, status: 'error', error: FAILED, durationMs: 0 });

describe('real audio in the player bar', () => {
  it('shows a file as it opens, plays and ends', async () => {
    const bridge = await openApp();

    bridge.emitPlayer(fileState({ status: 'loading', durationMs: 0 }));
    expect(await bar().findByText('Loading…')).toBeTruthy();
    expect(button('Pause').disabled).toBe(false);
    expect(slider('Seek').disabled).toBe(true);

    bridge.emitPlayer(fileState({ revision: 2 }));
    await waitFor(() => expect(bar().queryByText('Loading…')).toBeNull());
    expect(slider('Seek').disabled).toBe(false);

    bridge.emitPlayer(fileState({ revision: 3, status: 'ended', positionMs: 10_000 }));
    expect(await bar().findByText('Ended')).toBeTruthy();
    expect(slider('Seek').disabled).toBe(false);
    await fireEvent.click(button('Play'));
    expect(bridge.commands).toEqual([['togglePlay']]);
    expect(notice()).toBeNull();
  });

  it('disables previous and next', async () => {
    const bridge = await openApp();
    bridge.emitPlayer(fileState());
    await bar().findByText('tone');

    for (const name of ['Previous', 'Next']) {
      expect(button(name).getAttribute('aria-disabled')).toBe('true');
      await fireEvent.click(button(name));
    }

    expect(bridge.commands).toEqual([]);
  });

  it('toggles with Space and sends seek and volume', async () => {
    const bridge = await openApp();
    bridge.emitPlayer(fileState());
    await bar().findByText('tone');

    await fireEvent.keyDown(document.body, { key: ' ' });
    await fireEvent.change(slider('Seek'), { target: { value: '4000' } });
    await fireEvent.input(slider('Volume'), { target: { value: '20' } });

    expect(bridge.commands).toEqual([['togglePlay'], ['seek', 4000], ['setVolume', 20]]);
  });

  it('shows only the current track in the queue panel', async () => {
    const bridge = await openApp();
    bridge.emitPlayer(fileState());
    bridge.emit({ route: { view: 'home' }, queueOpen: true, revision: 1 });

    const panel = within(await screen.findByRole('complementary', { name: 'Queue' }));

    expect(panel.getByText('tone')).toBeTruthy();
    expect(panel.getByText('Nothing up next')).toBeTruthy();
    expect(panel.queryAllByRole('listitem')).toEqual([]);
  });
});

describe('the notice', () => {
  it('shows a failure that happened before the window was ready', async () => {
    const bridge = fakeBridge();
    bridge.snapshotState = failed();

    await openApp(bridge);

    expect((await screen.findByRole('alert')).textContent).toContain(FAILED);
    expect(bar().getByText('Could not play')).toBeTruthy();
    expect(button('Play').disabled).toBe(false);
    expect(slider('Seek').disabled).toBe(true);
  });

  it('shows a failure during playback, and can be dismissed for good', async () => {
    const bridge = await openApp();
    bridge.emitPlayer(fileState());
    bridge.emitPlayer(failed());
    await screen.findByRole('alert');

    await fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    await waitFor(() => expect(notice()).toBeNull());
    // The same failure, told again with a new volume.
    bridge.emitPlayer({ ...failed(3), volume: 10 });
    await waitFor(() => expect(slider('Volume').value).toBe('10'));

    expect(notice()).toBeNull();
  });

  it('goes away when playback is tried again', async () => {
    const bridge = await openApp();
    bridge.emitPlayer(failed());
    await screen.findByRole('alert');

    bridge.emitPlayer(fileState({ revision: 3, status: 'loading' }));

    await waitFor(() => expect(notice()).toBeNull());
  });

  it('says a sample track cannot be played and leaves playback alone', async () => {
    const bridge = await openApp();
    bridge.remembered = { query: '', filter: 'soundcloud' };
    bridge.emitPlayer(fileState());
    await bar().findByText('tone');
    bridge.failCommands = 'sample track, cannot be played';
    await fireEvent.click(screen.getByRole('link', { name: 'Search' }));

    await fireEvent.click(
      await screen.findByRole('button', { name: 'Play Harbor Lights by Mira Sol, Juno Park' }),
    );

    expect((await screen.findByRole('alert')).textContent).toContain(
      'sample track, cannot be played',
    );
    expect(screen.getByRole('alert').textContent).not.toContain('restart');
    expect(bar().getByText('tone')).toBeTruthy();
    expect(button('Pause')).toBeTruthy();
  });
});
