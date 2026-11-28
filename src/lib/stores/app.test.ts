import { describe, expect, it } from 'vitest';
import { fakeBridge, snapshot } from '../testing';
import { AppStore } from './app.svelte';

describe('AppStore', () => {
  it('subscribes before it reads the snapshot, so no change is lost', async () => {
    const bridge = fakeBridge();

    await new AppStore(bridge).connect();

    expect(bridge.calls).toEqual(['onNavigationChanged', 'getAppState']);
  });

  it('mirrors the snapshot from Rust', async () => {
    const store = new AppStore(fakeBridge());

    await store.connect();

    expect(store.navigation).toEqual(snapshot().navigation);
    expect(store.error).toBeNull();
  });

  it('changes route only when Rust says so', async () => {
    const bridge = fakeBridge({ announce: false });
    const store = new AppStore(bridge);
    await store.connect();

    await store.navigate({ view: 'library' });

    expect(bridge.requested).toEqual([{ view: 'library' }]);
    expect(store.navigation?.route).toEqual({ view: 'home' });

    bridge.emit({ route: { view: 'library' }, queueOpen: false, revision: 1 });

    expect(store.navigation?.route).toEqual({ view: 'library' });
  });

  it('ignores a snapshot that is older than an event it already has', async () => {
    const bridge = fakeBridge();
    bridge.beforeSnapshot = () => bridge.emit({ route: { view: 'search' }, queueOpen: false, revision: 4 });
    const store = new AppStore(bridge);

    await store.connect();

    expect(store.navigation).toEqual({ route: { view: 'search' }, queueOpen: false, revision: 4 });
  });

  it('keeps the message when Rust cannot be reached, and clears it on success', async () => {
    const bridge = fakeBridge();
    const store = new AppStore(bridge);
    await store.connect();
    bridge.failNavigate = 'there is no playlist with the id `gone`';

    await store.navigate({ view: 'playlist', id: 'gone' });

    expect(store.error).toBe('there is no playlist with the id `gone`');

    bridge.failNavigate = null;
    await store.navigate({ view: 'search' });

    expect(store.error).toBeNull();
  });

  it('reports a failed start', async () => {
    const bridge = fakeBridge();
    bridge.failSnapshot = new Error('ipc unavailable');
    const store = new AppStore(bridge);

    await store.connect();

    expect(store.error).toBe('ipc unavailable');
    expect(store.navigation).toBeNull();
  });

  it('stops listening when disconnected', async () => {
    const bridge = fakeBridge();
    const store = new AppStore(bridge);
    const disconnect = await store.connect();

    disconnect();

    expect(bridge.listeners).toBe(0);
  });
});
