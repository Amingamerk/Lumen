// Test double for the Rust side. Only imported by tests.
import type {
  AppSnapshot,
  Bridge,
  CollectionDetail,
  CollectionSummary,
  Library,
  NavigationState,
  Route,
  SearchView,
  Shelf,
  SourceFilter,
  TrackRow,
  TrackSlice,
} from './ipc';

export function snapshot(): AppSnapshot {
  return {
    navigation: { route: { view: 'home' }, revision: 0 },
    playlists: [
      { id: 'night-shift', name: 'Night shift', source: 'youtube', badge: 'YT' },
      { id: 'tunnel-mixes', name: 'Tunnel mixes', source: 'soundcloud', badge: 'SC' },
      { id: 'liked-on-spotify', name: 'Liked on Spotify', source: 'spotify', badge: 'SP' },
    ],
  };
}

export function library(): Library {
  return {
    playlists: [
      {
        id: 'night-shift',
        kind: 'playlist',
        title: 'Night shift',
        subtitle: 'Slow songs for after midnight',
        source: 'youtube',
        badge: 'YT',
      },
      {
        id: 'tunnel-mixes',
        kind: 'playlist',
        title: 'Tunnel mixes',
        subtitle: 'Long, low and echoing',
        source: 'soundcloud',
        badge: 'SC',
      },
      {
        id: 'liked-on-spotify',
        kind: 'playlist',
        title: 'Liked on Spotify',
        subtitle: 'Your liked songs, imported',
        source: 'spotify',
        badge: 'SP',
      },
    ],
    albums: [
      {
        id: 'low-tide',
        kind: 'album',
        title: 'Low Tide',
        subtitle: 'Mira Sol',
        source: 'youtube',
        badge: 'YT',
      },
    ],
  };
}

function collection(id: string): CollectionSummary {
  const { playlists, albums } = library();
  const found = [...playlists, ...albums].find((item) => item.id === id);
  // Rust rejects with a plain string.
  if (!found) throw `no playlist or album has the id “${id}”`;
  return found;
}

export function home(): Shelf[] {
  return [
    {
      id: 'jump-back-in',
      title: 'Jump back in',
      items: [collection('night-shift'), collection('low-tide')],
    },
    { id: 'soundcloud', title: 'From SoundCloud', items: [collection('tunnel-mixes')] },
  ];
}

export function tracks(): TrackRow[] {
  return [
    {
      id: 't1',
      title: 'Harbor Lights',
      artists: ['Mira Sol', 'Juno Park'],
      album: 'Low Tide',
      durationMs: 201_000,
      sources: [{ source: 'youtube', badge: 'YT' }],
    },
    {
      id: 't2',
      title: 'Lamplighter',
      artists: ['Juno Park'],
      album: null,
      durationMs: 189_000,
      sources: [
        { source: 'spotify', badge: 'SP' },
        { source: 'youtube', badge: 'YT' },
      ],
    },
  ];
}

const LENGTH_MS = 201_000;

/** The rows Rust would send for this window of a list of `total` numbered Tracks. */
export function trackSlice(offset: number, limit: number, total: number): TrackSlice {
  const length = Math.max(0, Math.min(limit, total - offset));
  return {
    total,
    offset,
    tracks: Array.from({ length }, (_, at) => ({
      ...tracks()[0],
      id: `n${offset + at + 1}`,
      title: `Track ${offset + at + 1}`,
    })),
  };
}

/** The same results whatever is asked: matching is Rust's job, and tested there. */
export function searchView(query: string, filter: SourceFilter): SearchView {
  return { query, filter, tracks: tracks(), collections: [collection('low-tide')] };
}

/** A promise the test settles when it chooses, to put answers out of order. */
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

type Answer = (query: string, filter: SourceFilter) => SearchView | Promise<SearchView>;

/** Records what the UI asks for and, unless told not to, answers like Rust does. */
export function fakeBridge({ announce = true } = {}) {
  const handlers = new Set<(navigation: NavigationState) => void>();
  let revision = 0;

  function catalogCall(name: string) {
    bridge.calls.push(name);
    if (bridge.failCatalog) throw bridge.failCatalog;
  }

  const bridge = {
    calls: [] as string[],
    requested: [] as Route[],
    searches: [] as { query: string; filter: SourceFilter }[],
    remembered: { query: '', filter: 'all' as SourceFilter },
    answer: searchView as Answer,
    failNavigate: null as string | null,
    failSnapshot: null as Error | null,
    failCatalog: null as string | null,
    beforeSnapshot: null as (() => void) | null,
    beforeCollection: null as ((id: string) => Promise<void> | undefined) | null,
    beforeRemembered: null as (() => Promise<void>) | null,
    beforeSlice: null as (() => Promise<void>) | null,
    slices: [] as { id: string; offset: number; limit: number }[],
    failSlices: null as string | null,
    /** How many Tracks a Collection holds, where it is not two. */
    trackCounts: {} as Record<string, number>,
    playlists: snapshot().playlists,
    shelves: home(),
    saved: library(),
    get listeners() {
      return handlers.size;
    },
    emit(navigation: NavigationState) {
      for (const handler of handlers) handler(navigation);
    },
    async getAppState(): Promise<AppSnapshot> {
      bridge.calls.push('getAppState');
      if (bridge.failSnapshot) throw bridge.failSnapshot;
      bridge.beforeSnapshot?.();
      return { ...snapshot(), playlists: bridge.playlists };
    },
    async navigate(route: Route) {
      bridge.requested.push(route);
      // Rust rejects with a plain string.
      if (bridge.failNavigate) throw bridge.failNavigate;
      if (announce) bridge.emit({ route, revision: ++revision });
    },
    async getHome(): Promise<Shelf[]> {
      catalogCall('getHome');
      return bridge.shelves;
    },
    async getLibrary(): Promise<Library> {
      catalogCall('getLibrary');
      return bridge.saved;
    },
    async getCollection(id: string): Promise<CollectionDetail> {
      catalogCall('getCollection');
      await bridge.beforeCollection?.(id);
      const trackCount = bridge.trackCounts[id] ?? 2;
      return { ...collection(id), trackCount, durationMs: trackCount * LENGTH_MS };
    },
    async getTrackSlice(id: string, offset: number, limit: number): Promise<TrackSlice> {
      catalogCall('getTrackSlice');
      bridge.slices.push({ id, offset, limit });
      if (bridge.failSlices) throw bridge.failSlices;
      await bridge.beforeSlice?.();
      collection(id);
      return trackSlice(offset, limit, bridge.trackCounts[id] ?? 2);
    },
    async getSearch(): Promise<SearchView> {
      catalogCall('getSearch');
      const { query, filter } = bridge.remembered;
      await bridge.beforeRemembered?.();
      return bridge.answer(query, filter);
    },
    async searchCatalog(query: string, filter: SourceFilter): Promise<SearchView> {
      catalogCall('searchCatalog');
      bridge.searches.push({ query, filter });
      bridge.remembered = { query, filter };
      return bridge.answer(query, filter);
    },
    async onNavigationChanged(handler: (navigation: NavigationState) => void) {
      bridge.calls.push('onNavigationChanged');
      handlers.add(handler);
      return () => {
        handlers.delete(handler);
      };
    },
  } satisfies Bridge & Record<string, unknown>;

  return bridge;
}
