// Test double for the Rust side. Only imported by tests.
import type { AppSnapshot, Bridge, NavigationState, Route } from './ipc';

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

/** Records what the UI asks for and, unless told not to, answers like Rust does. */
export function fakeBridge({ announce = true } = {}) {
  const handlers = new Set<(navigation: NavigationState) => void>();
  let revision = 0;

  const bridge = {
    calls: [] as string[],
    requested: [] as Route[],
    failNavigate: null as string | null,
    failSnapshot: null as Error | null,
    beforeSnapshot: null as (() => void) | null,
    playlists: snapshot().playlists,
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
