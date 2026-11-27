// The only door to Rust. Types mirror src-tauri/src/state.rs and src-tauri/src/catalog/.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type SourceKind = 'youtube' | 'soundcloud' | 'spotify';

export interface SourceTag {
  source: SourceKind;
  badge: 'YT' | 'SC' | 'SP';
}

export interface PlaylistSummary extends SourceTag {
  id: string;
  name: string;
}

/** A playlist or album as a card or a page header shows it. */
export interface CollectionSummary extends SourceTag {
  id: string;
  kind: 'playlist' | 'album';
  title: string;
  /** A playlist's description, or an album's artists. */
  subtitle: string;
}

export interface TrackRow {
  id: string;
  title: string;
  artists: string[];
  album: string | null;
  durationMs: number;
  sources: SourceTag[];
}

/** One horizontal row of cards on Home. */
export interface Shelf {
  id: string;
  title: string;
  items: CollectionSummary[];
}

export interface Library {
  playlists: CollectionSummary[];
  albums: CollectionSummary[];
}

export type SourceFilter = 'all' | 'youtube' | 'soundcloud';

/** A search and what Rust found for it. */
export interface SearchView {
  query: string;
  filter: SourceFilter;
  tracks: TrackRow[];
  collections: CollectionSummary[];
}

export type Route =
  | { view: 'home' }
  | { view: 'search' }
  | { view: 'library' }
  | { view: 'playlist'; id: string };

export interface NavigationState {
  route: Route;
  revision: number;
}

export interface AppSnapshot {
  navigation: NavigationState;
  playlists: PlaylistSummary[];
}

export interface Bridge {
  getAppState(): Promise<AppSnapshot>;
  navigate(route: Route): Promise<void>;
  getHome(): Promise<Shelf[]>;
  getLibrary(): Promise<Library>;
  getCollection(id: string): Promise<CollectionSummary>;
  /** The search Rust remembers, with its results. */
  getSearch(): Promise<SearchView>;
  searchCatalog(query: string, filter: SourceFilter): Promise<SearchView>;
  /** Resolves with a function that stops listening. */
  onNavigationChanged(handler: (navigation: NavigationState) => void): Promise<() => void>;
}

/** Must match NAVIGATION_CHANGED in src-tauri/src/ipc/mod.rs. */
const NAVIGATION_CHANGED = 'navigation-changed';

export const tauriBridge: Bridge = {
  getAppState: () => invoke<AppSnapshot>('get_app_state'),
  navigate: (route) => invoke('navigate', { route }),
  getHome: () => invoke<Shelf[]>('get_home'),
  getLibrary: () => invoke<Library>('get_library'),
  getCollection: (id) => invoke<CollectionSummary>('get_collection', { id }),
  getSearch: () => invoke<SearchView>('get_search'),
  searchCatalog: (query, filter) => invoke<SearchView>('search_catalog', { query, filter }),
  onNavigationChanged: (handler) =>
    listen<NavigationState>(NAVIGATION_CHANGED, (event) => handler(event.payload)),
};
