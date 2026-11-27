import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import App from './App.svelte';
import { AppStore } from './lib/stores/app.svelte';
import { CatalogStore } from './lib/stores/catalog.svelte';
import { fakeBridge } from './lib/testing';

const stores = (bridge: ReturnType<typeof fakeBridge>) => ({
  store: new AppStore(bridge),
  catalog: new CatalogStore(bridge),
});

async function openApp(bridge = fakeBridge()) {
  const view = render(App, { props: stores(bridge) });
  // Home cards name playlists too, so wait for the sidebar's own link.
  await screen.findByRole('link', { name: 'Night shift, from YouTube Music' });
  return { ...view, bridge };
}

const sidebar = () => within(screen.getByRole('navigation', { name: 'Main' }));
const pageTitle = () => screen.getByRole('heading', { level: 1 }).textContent?.trim();

describe('app shell', () => {
  it('shows the brand, the three destinations and the player frame', async () => {
    await openApp();

    expect(sidebar().getByRole('img', { name: 'Lumen' })).toBeTruthy();
    // The wordmark is the name; no text repeats it beside the image.
    expect(sidebar().queryByText('Lumen')).toBeNull();
    for (const name of ['Home', 'Search', 'Library']) {
      expect(sidebar().getByRole('link', { name })).toBeTruthy();
    }
    expect(screen.getByRole('region', { name: 'Player' })).toBeTruthy();
    expect(screen.getByRole('main')).toBeTruthy();
  });

  it('lists the playlists from Rust with a badge and a spoken source name', async () => {
    await openApp();

    const expected = [
      ['Night shift', 'YT', 'YouTube\u00a0Music'],
      ['Tunnel mixes', 'SC', 'SoundCloud'],
      ['Liked on Spotify', 'SP', 'Spotify'],
    ];
    for (const [name, badge, source] of expected) {
      const link = sidebar().getByRole('link', { name: `${name}, from ${source}` });
      expect(within(link).getByText(badge)).toBeTruthy();
    }
  });

  it('says so when there are no playlists yet', async () => {
    const bridge = fakeBridge();
    bridge.playlists = [];

    render(App, { props: stores(bridge) });

    expect(await sidebar().findByText('No playlists yet')).toBeTruthy();
  });

  it('asks Rust to switch view and shows what Rust reports', async () => {
    const { bridge } = await openApp();
    expect(pageTitle()).toBe('Home');

    for (const name of ['Search', 'Library', 'Home']) {
      await fireEvent.click(sidebar().getByRole('link', { name }));

      await waitFor(() => expect(pageTitle()).toBe(name));
      expect(sidebar().getByRole('link', { name }).getAttribute('aria-current')).toBe('page');
      expect(screen.getAllByRole('link', { current: 'page' })).toHaveLength(1);
    }
    expect(bridge.requested).toEqual([{ view: 'search' }, { view: 'library' }, { view: 'home' }]);
  });

  it('opens a playlist frame', async () => {
    const { bridge } = await openApp();

    await fireEvent.click(sidebar().getByRole('link', { name: /tunnel mixes/i }));

    await waitFor(() => expect(pageTitle()).toBe('Tunnel mixes'));
    expect(bridge.requested).toEqual([{ view: 'playlist', id: 'tunnel-mixes' }]);
    expect(screen.getByRole('main').textContent).toContain('From SoundCloud');
  });

  it('keeps the player controls inert until playback exists', async () => {
    await openApp();
    const player = within(screen.getByRole('region', { name: 'Player' }));

    expect(player.getByText('Nothing is playing')).toBeTruthy();
    for (const name of ['Previous', 'Play', 'Next']) {
      expect(player.getByRole<HTMLButtonElement>('button', { name }).disabled).toBe(true);
    }
  });

  it('never uses an HTML audio element', async () => {
    const { container } = await openApp();

    expect(container.querySelector('audio, video')).toBeNull();
  });

  it('says what went wrong when Rust rejects a request', async () => {
    const { bridge } = await openApp();
    bridge.failNavigate = 'could not notify the window';

    await fireEvent.click(sidebar().getByRole('link', { name: 'Search' }));

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('could not notify the window');
    expect(alert.textContent).toContain('Try again');
  });

  it('explains a failed start instead of showing an empty window', async () => {
    const bridge = fakeBridge();
    bridge.failSnapshot = new Error('ipc unavailable');

    render(App, { props: stores(bridge) });

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('ipc unavailable');
    expect(alert.textContent).toContain('Restart Lumen');
  });

  it('stops listening when the window content goes away', async () => {
    const { bridge, unmount } = await openApp();

    unmount();

    await waitFor(() => expect(bridge.listeners).toBe(0));
  });
});
