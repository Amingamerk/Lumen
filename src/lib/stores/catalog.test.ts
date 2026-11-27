import { describe, expect, it } from 'vitest';
import type { SearchView } from '../ipc';
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

  it('sends the query and chip to Rust and shows its answer unfiltered', async () => {
    const { bridge, catalog } = setUp();

    await catalog.runSearch('no such words', 'soundcloud');

    expect(bridge.searches).toEqual([{ query: 'no such words', filter: 'soundcloud' }]);
    expect(catalog.search).toEqual(searchView('no such words', 'soundcloud'));
  });

  it('keeps the newest results when an older search answers late', async () => {
    const { bridge, catalog } = setUp();
    const slow = deferred<SearchView>();
    bridge.answer = (query, filter) => (query === 'n' ? slow.promise : searchView(query, filter));

    const first = catalog.runSearch('n', 'all');
    await catalog.runSearch('ni', 'all');
    slow.resolve(searchView('n', 'all'));
    await first;

    expect(catalog.search?.query).toBe('ni');
  });

  it('does not let the remembered search overwrite what was typed meanwhile', async () => {
    const { bridge, catalog } = setUp();
    const slow = deferred<undefined>();
    bridge.beforeRemembered = () => slow.promise;

    const opening = catalog.openSearch();
    await catalog.runSearch('ferry', 'all');
    slow.resolve(undefined);
    await opening;

    expect(catalog.search?.query).toBe('ferry');
  });

  it('ignores the failure of a search that has been replaced', async () => {
    const { bridge, catalog } = setUp();
    const slow = deferred<SearchView>();
    bridge.answer = (query, filter) => (query === 'n' ? slow.promise : searchView(query, filter));

    const first = catalog.runSearch('n', 'all');
    await catalog.runSearch('ni', 'all');
    slow.reject('too late to matter');
    await first;

    expect(catalog.error).toBeNull();
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
