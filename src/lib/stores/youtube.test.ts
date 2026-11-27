import { describe, expect, it, vi } from 'vitest';
import { deferred, fakeBridge, serviceStatus } from '../testing';
import { YoutubeStore } from './youtube.svelte';

async function connected(bridge = fakeBridge()) {
  const youtube = new YoutubeStore(bridge);
  const disconnect = await youtube.connect();
  return { youtube, bridge, disconnect };
}

describe('youtube store', () => {
  it('listens before it asks, then shows what Rust answers', async () => {
    const bridge = fakeBridge();
    bridge.youtubeStatus = serviceStatus({ revision: 4, check: 'verified' });

    const { youtube } = await connected(bridge);

    expect(bridge.calls).toEqual(['onYoutubeStatusChanged', 'getYoutubeStatus']);
    expect(youtube.status).toEqual(bridge.youtubeStatus);
  });

  it('drops a status older than the one it shows', async () => {
    const { youtube, bridge } = await connected();

    bridge.emitYoutube(serviceStatus({ revision: 5, check: 'verified' }));
    bridge.emitYoutube(serviceStatus({ revision: 3, check: 'offline' }));

    expect(youtube.status?.check).toBe('verified');
  });

  it('keeps a change that overtook the first answer', async () => {
    const bridge = fakeBridge();
    const answer = deferred<undefined>();
    bridge.beforeYoutubeStatus = () => answer.promise;
    const youtube = new YoutubeStore(bridge);
    const connecting = youtube.connect();
    await vi.waitFor(() => expect(bridge.calls).toContain('getYoutubeStatus'));

    bridge.emitYoutube(serviceStatus({ revision: 1, check: 'checking' }));
    answer.resolve(undefined);
    await connecting;

    expect(youtube.status?.check).toBe('checking');
  });

  it('stops listening when asked', async () => {
    const { bridge, disconnect } = await connected();

    disconnect();

    expect(bridge.youtubeListeners).toBe(0);
  });

  it('asks Rust to resume and waits for its word', async () => {
    const bridge = fakeBridge();
    bridge.youtubeStatus = serviceStatus({ gate: { state: 'paused', reason: 'botCheck' } });
    const { youtube } = await connected(bridge);

    await youtube.resume();

    expect(bridge.commands).toEqual([['resumeYoutube']]);
    expect(youtube.status).toEqual(bridge.youtubeStatus);
  });

  it('says why Resume failed, until one works', async () => {
    const { youtube, bridge } = await connected();
    bridge.failCommands = 'the window failed';

    await youtube.resume();
    expect(youtube.error).toBe('the window failed');

    bridge.failCommands = null;
    await youtube.resume();
    expect(youtube.error).toBeNull();
  });
});
