// The player bar and the clicks that start playback, through the whole app.
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import { appStores, deferred, fakeBridge, playerState, trackSlice } from './lib/testing';

async function openApp(bridge = fakeBridge()) {
  const view = render(App, { props: appStores(bridge) });
  await screen.findByRole('link', { name: /^night shift, playlist$/i });
  await waitFor(() => expect(bridge.calls).toContain('getPlayerState'));
  return { ...view, bridge };
}

const bar = () => within(screen.getByRole('region', { name: 'Player' }));
const main = () => within(screen.getByRole('main'));
const button = (name: string) => bar().getByRole<HTMLButtonElement>('button', { name });
const slider = (name: string) => bar().getByRole<HTMLInputElement>('slider', { name });

async function playing(overrides: Parameters<typeof playerState>[0] = {}) {
  const opened = await openApp();
  opened.bridge.emitPlayer(playerState(overrides));
  await bar().findByText('Harbor Lights');
  return opened;
}

describe('player bar', () => {
  it('shows an idle player', async () => {
    await openApp();

    expect(bar().getByText('Nothing is playing')).toBeTruthy();
    for (const name of ['Previous', 'Play', 'Next']) expect(button(name).disabled).toBe(true);
    await waitFor(() => expect(slider('Volume').disabled).toBe(false));
    expect(slider('Seek').disabled).toBe(true);
    expect(slider('Volume').getAttribute('aria-valuetext')).toBe('80%');
  });

  it('shows the current track, the clock and the length', async () => {
    const { bridge } = await playing();

    bridge.emitPosition({ revision: 1, positionMs: 65_000 });

    expect(bar().getByText('Mira Sol, Juno Park')).toBeTruthy();
    await waitFor(() =>
      expect(slider('Seek').getAttribute('aria-valuetext')).toBe('1:05 of 3:21'),
    );
    expect(bar().getByText('1:05')).toBeTruthy();
    expect(bar().getByText('3:21')).toBeTruthy();
  });

  it('labels Play by the status and sends the transport to Rust', async () => {
    const { bridge } = await playing();

    await fireEvent.click(button('Pause'));
    await fireEvent.click(button('Previous'));
    await fireEvent.click(button('Next'));
    expect(bridge.commands).toEqual([['togglePlay'], ['previousTrack'], ['nextTrack']]);

    bridge.emitPlayer(playerState({ revision: 2, status: 'paused' }));
    await waitFor(() => expect(button('Play')).toBeTruthy());
  });

  it('marks Next aria-disabled without a next entry', async () => {
    const { bridge } = await playing({ canNext: false });

    expect(button('Next').getAttribute('aria-disabled')).toBe('true');
    expect(button('Next').disabled).toBe(false);
    await fireEvent.click(button('Next'));

    expect(bridge.commands).toEqual([]);
  });

  it('seeks once, on release', async () => {
    const { bridge } = await playing();

    await fireEvent.input(slider('Seek'), { target: { value: '30000' } });
    await fireEvent.input(slider('Seek'), { target: { value: '60000' } });
    expect(bridge.commands).toEqual([]);
    await fireEvent.change(slider('Seek'), { target: { value: '60000' } });

    expect(bridge.commands).toEqual([['seek', 60_000]]);
  });

  it('does not let ticks move a dragged thumb', async () => {
    const { bridge } = await playing();

    await fireEvent.input(slider('Seek'), { target: { value: '90000' } });
    bridge.emitPosition({ revision: 1, positionMs: 2000 });
    await Promise.resolve();
    expect(slider('Seek').value).toBe('90000');

    await fireEvent.blur(slider('Seek'));
    await waitFor(() => expect(slider('Seek').value).toBe('2000'));
  });

  it('holds the thumb until Rust has answered a seek', async () => {
    const { bridge } = await playing();
    const reply = deferred<unknown>();
    bridge.beforeReply = async () => void (await reply.promise);

    await fireEvent.input(slider('Seek'), { target: { value: '90000' } });
    await fireEvent.change(slider('Seek'), { target: { value: '90000' } });
    expect(slider('Seek').value).toBe('90000');

    bridge.emitPlayer(playerState({ revision: 2, positionMs: 90_000 }));
    reply.resolve(null);
    await waitFor(() => expect(slider('Seek').value).toBe('90000'));
  });

  it('sends the volume to Rust', async () => {
    const { bridge } = await playing();

    await fireEvent.input(slider('Volume'), { target: { value: '35' } });

    expect(bridge.commands).toEqual([['setVolume', 35]]);
  });

  it('asks Rust to toggle the queue', async () => {
    const { bridge } = await openApp();

    await fireEvent.click(button('Queue'));

    expect(bridge.commands).toEqual([['toggleQueue']]);
    expect(button('Queue').getAttribute('aria-expanded')).toBe('false');
  });

  it('says why playback failed', async () => {
    const { bridge } = await playing();
    bridge.failCommands = 'track is not in the last search';

    await fireEvent.click(button('Pause'));

    expect((await screen.findByRole('alert')).textContent).toContain(
      'track is not in the last search',
    );
  });

  it('stops listening to the player when the window content goes away', async () => {
    const { bridge, unmount } = await openApp();

    unmount();

    await waitFor(() => expect(bridge.playerListeners).toBe(0));
  });
});

describe('playing from a list', () => {
  it('sends a search result to Rust and the bar follows the event', async () => {
    const { bridge } = await openApp();
    // Only the sample catalog can be played from Search so far.
    bridge.emitAccount('connected');
    bridge.remembered = { query: '', filter: 'all' };
    await fireEvent.click(screen.getByRole('link', { name: 'Search' }));

    await fireEvent.click(
      await main().findByRole('button', { name: 'Play Harbor Lights by Mira Sol, Juno Park' }),
    );

    expect(bridge.commands).toEqual([['playSearchResult', 't1']]);
    expect(bar().queryByText('Harbor Lights')).toBeNull();
    bridge.emitPlayer(playerState());
    expect(await bar().findByText('Harbor Lights')).toBeTruthy();
  });

  it('plays the song of a playlist row far down the list, by its id', async () => {
    const bridge = fakeBridge();
    bridge.lists[0].entries = trackSlice(0, 5000, 5000).tracks;
    await openApp(bridge);
    await fireEvent.click(screen.getByRole('link', { name: /^night shift, playlist$/i }));
    await main().findByRole('button', { name: /^Play Track 1 by/ });

    const scroller = screen.getByRole('main').querySelector<HTMLElement>('[data-scroller]') as HTMLElement;
    Object.defineProperty(scroller, 'clientHeight', { value: 560, configurable: true });
    main().getAllByRole('rowgroup')[1].getBoundingClientRect = () => new DOMRect(0, -3999 * 56);
    await fireEvent.scroll(scroller);
    await fireEvent.click(await main().findByRole('button', { name: /^Play Track 4000 by/ }));

    expect(bridge.commands).toEqual([['playLibraryTrack', 'n4000']]);
  });
});
