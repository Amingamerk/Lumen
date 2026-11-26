// The only door to Rust. Types mirror src-tauri/src/state.rs.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type SourceKind = 'youtube' | 'soundcloud' | 'spotify';

export interface PlaylistSummary {
  id: string;
  name: string;
  source: SourceKind;
  badge: 'YT' | 'SC' | 'SP';
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
  /** Resolves with a function that stops listening. */
  onNavigationChanged(handler: (navigation: NavigationState) => void): Promise<() => void>;
}

/** Must match NAVIGATION_CHANGED in src-tauri/src/ipc/mod.rs. */
const NAVIGATION_CHANGED = 'navigation-changed';

export const tauriBridge: Bridge = {
  getAppState: () => invoke<AppSnapshot>('get_app_state'),
  navigate: (route) => invoke('navigate', { route }),
  onNavigationChanged: (handler) =>
    listen<NavigationState>(NAVIGATION_CHANGED, (event) => handler(event.payload)),
};
