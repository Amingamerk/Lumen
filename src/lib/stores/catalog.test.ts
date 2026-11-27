import { describe, expect, it } from 'vitest';
import { deferred, fakeBridge, home, library, searchView } from '../testing';
import { CatalogStore } from './catalog.svelte';

function setUp() {
  const bridge = fakeBridge();
  return { bridge, catalog: new CatalogStore(bridge) };
}

describe('CatalogStore', () => {
  it('knows nothing before it asks Rust', () => {
    const { bridge, catalog } = setUp();

    expect([catalog.home, catalog.library, catalog.search, catalog.collection]).toEqual([
      null,
      null,
      null,
      null,
    ]);
    expect(bridge.calls).toEqual([]);
  });

  it('mirrors the Home shelves and the Library, asking once each', async () => {
    const { bridge, catalog } = setUp();

    await catalog.loadHome();
    await catalog.loadLibrary();

    expect(catalog.home).toEqual(home());
    expect(catalog.library).toEqual(library());
    expect(bridge.calls).toEqual(['getHome', 'getLibrary']);
  });

  it('mirrors the collection behind a playlist route', async () => {
    const { catalog } = setUp();

    await catalog.loadCollection('low-tide');

    expect(catalog.collection?.title).toBe('Low Tide');
  });

  it('keeps the newest collection when an older answer arrives late', async () => {
    const { bridge, catalog } = setUp();
    const slow = deferred<undefined>();
    bridge.beforeCollection = (id) => (id === 'low-tide' ? slow.promise : undefined);

    const first = catalog.loadCollection('low-tide');
    await catalog.loadCollection('night-shift');
    slow.resolve(undefined);
    await first;

    expect(catalog.collection?.id).toBe('night-shift');
  });

  it('opens the search Rust remembers', async () => {
    const { bridge, catalog } = setUp();
    bridge.remembered = { query: 'ferry', filter: 'soundcloud' };

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
    expect(catalog.home).toEqual(home());

    bridge.failCatalog = null;
    await catalog.loadLibrary();

    expect(catalog.error).toBeNull();
  });
});
