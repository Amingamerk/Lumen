import { describe, expect, it, vi } from 'vitest';
import { deferred, fakeBridge, searchView, tracks } from '../testing';
import { CatalogStore } from './catalog.svelte';

function setUp() {
  const bridge = fakeBridge();
  return { bridge, catalog: new CatalogStore(bridge) };
}

describe('CatalogStore', () => {
  it('knows nothing before it asks Rust', () => {
    const { bridge, catalog } = setUp();

    expect([catalog.home, catalog.search]).toEqual([null, null]);
    expect(bridge.calls).toEqual([]);
  });

  it('mirrors the Home shelves, asking once', async () => {
    const { bridge, catalog } = setUp();
    bridge.played = tracks();

    await catalog.loadHome();

    expect(catalog.home?.recent.map((row) => row.id)).toEqual(tracks().map((row) => row.id));
    expect(catalog.home?.liked).toEqual([]);
    expect(catalog.home?.playlists.map((list) => list.name)).toEqual(['Night shift']);
    expect(bridge.calls).toEqual(['getHome']);
  });

  it('has no cards on Home for an empty library', async () => {
    const { bridge, catalog } = setUp();
    bridge.lists = [];

    await catalog.loadHome();

    expect(catalog.home).toEqual({ recent: [], liked: [], playlists: [] });
  });

  describe('Home after a library event', () => {
    it('shows a new like after one event', async () => {
      const { bridge, catalog } = setUp();
      await catalog.connect();
      await catalog.loadHome();

      await bridge.likeTrack('t1');

      await vi.waitFor(() => expect(catalog.home?.liked.map((row) => row.id)).toEqual(['t1']));
      expect(bridge.calls.filter((call) => call === 'getHome')).toHaveLength(2);
    });

    it('shows a new play after one event', async () => {
      const { bridge, catalog } = setUp();
      await catalog.connect();
      await catalog.loadHome();

      bridge.played = [tracks()[1]];
      bridge.emitHistory({ revision: 1, playedCount: 1 });

      await vi.waitFor(() => expect(catalog.home?.recent.map((row) => row.id)).toEqual(['t2']));
    });

    it('shows a new playlist after one event', async () => {
      const { bridge, catalog } = setUp();
      await catalog.connect();
      await catalog.loadHome();

      await bridge.createPlaylist('Mix');

      await vi.waitFor(() =>
        expect(catalog.home?.playlists.map((list) => list.name)).toEqual(['Night shift', 'Mix']),
      );
    });

    it('does not ask for Home before Home was opened', async () => {
      const { bridge, catalog } = setUp();
      await catalog.connect();

      await bridge.likeTrack('t1');

      expect(bridge.calls).not.toContain('getHome');
    });
  });

  it('opens the search Rust remembers', async () => {
    const { bridge, catalog } = setUp();
    bridge.remembered = { query: 'ferry', filter: 'soundcloud', kind: 'all' };

    await catalog.openSearch();

    expect(catalog.search).toEqual(searchView('ferry', 'soundcloud'));
  });

  it('submits to Rust and mirrors the event that answers', async () => {
    const { bridge, catalog } = setUp();
    await catalog.connect();

    await catalog.submitSearch('no such words', 'youtube');

    expect(bridge.searches).toEqual([{ query: 'no such words', filter: 'youtube' }]);
    expect(catalog.search).toEqual(searchView('no such words', 'youtube', { revision: 1 }));
  });

  it('tells Rust which chip was chosen without submitting', async () => {
    const { bridge, catalog } = setUp();
    await catalog.connect();

    await catalog.setSearchFilter('soundcloud');

    expect(bridge.filterChanges).toEqual(['soundcloud']);
    expect(bridge.searches).toEqual([]);
    expect(catalog.search?.filter).toBe('soundcloud');
  });

  it('drops a view older than the one it has', async () => {
    const { bridge, catalog } = setUp();
    await catalog.connect();

    bridge.emitSearch(searchView('new', 'all', { revision: 5 }));
    bridge.emitSearch(searchView('old', 'all', { revision: 4 }));

    expect(catalog.search?.query).toBe('new');
  });

  it('does not let the remembered search overwrite a newer event', async () => {
    const { bridge, catalog } = setUp();
    await catalog.connect();
    const slow = deferred<undefined>();
    bridge.beforeRemembered = () => slow.promise;

    const opening = catalog.openSearch();
    await catalog.submitSearch('ferry', 'all');
    slow.resolve(undefined);
    await opening;

    expect(catalog.search?.query).toBe('ferry');
  });

  it('stops listening for search changes', async () => {
    const { bridge, catalog } = setUp();
    const disconnect = await catalog.connect();
    expect(bridge.searchListeners).toBe(1);

    disconnect();

    expect(bridge.searchListeners).toBe(0);
  });

  it('says why a search command failed', async () => {
    const { bridge, catalog } = setUp();
    bridge.failCatalog = 'search unavailable';

    await catalog.submitSearch('ferry', 'all');

    expect(catalog.error).toBe('search unavailable');
  });

  it('keeps what it has and says why when Rust fails, then recovers', async () => {
    const { bridge, catalog } = setUp();
    await catalog.loadHome();
    bridge.failCatalog = 'catalog unavailable';

    await catalog.loadHome();

    expect(catalog.error).toBe('catalog unavailable');
    expect(catalog.home).not.toBeNull();

    bridge.failCatalog = null;
    await catalog.loadHome();

    expect(catalog.error).toBeNull();
  });
});
