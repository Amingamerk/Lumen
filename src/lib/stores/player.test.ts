import { afterEach, describe, expect, it, vi } from 'vitest';
import { fakeBridge, playerState } from '../testing';
import { PlayerStore } from './player.svelte';

async function connected(bridge = fakeBridge()) {
  const player = new PlayerStore(bridge);
  const disconnect = await player.connect();
  return { bridge, player, disconnect };
}

afterEach(() => vi.useRealTimers());

describe('player store', () => {
  it('subscribes before the snapshot', async () => {
    const { bridge, player } = await connected();

    expect(bridge.calls).toEqual(['onPlayerChanged', 'onPlayerPosition', 'getPlayerState']);
    expect(player.state?.status).toBe('idle');
  });

  it('mirrors only what Rust sends', async () => {
    const { bridge, player } = await connected();
    const before = player.state;

    await player.togglePlay();

    expect(player.state).toBe(before);
    bridge.emitPlayer(playerState({ revision: 1, positionMs: 4000 }));
    expect(player.state?.status).toBe('playing');
    expect(player.positionMs).toBe(4000);
  });

  it('sends every command with its arguments', async () => {
    const { bridge, player } = await connected();

    await player.playCollection('night-shift', 2);
    await player.playSearchResult('t1');
    await player.togglePlay();
    await player.next();
    await player.previous();
    await player.seek(5000);
    await player.seekBy(-5000);
    await player.setVolume(35);
    await player.changeVolume(5);
    await player.moveQueueEntry(13, null);

    expect(bridge.commands).toEqual([
      ['playCollection', 'night-shift', 2],
      ['playSearchResult', 't1'],
      ['togglePlay'],
      ['nextTrack'],
      ['previousTrack'],
      ['seek', 5000],
      ['seekBy', -5000],
      ['setVolume', 35],
      ['changeVolume', 5],
      ['moveQueueEntry', 13, null],
    ]);
  });

  it('drops an older snapshot or event', async () => {
    const bridge = fakeBridge();
    const player = new PlayerStore(bridge);
    bridge.snapshotState = playerState({ revision: 3 });
    bridge.getPlayerState = async () => {
      bridge.emitPlayer(playerState({ revision: 4, volume: 10 }));
      return bridge.snapshotState;
    };

    await player.connect();
    bridge.emitPlayer(playerState({ revision: 3, volume: 20 }));

    expect(player.state?.revision).toBe(4);
    expect(player.state?.volume).toBe(10);
  });

  it('applies a position only for its revision', async () => {
    const { bridge, player } = await connected();
    bridge.emitPlayer(playerState({ revision: 5, positionMs: 1000 }));

    bridge.emitPosition({ revision: 4, positionMs: 9000 });
    bridge.emitPosition({ revision: 6, positionMs: 9000 });
    expect(player.positionMs).toBe(1000);

    bridge.emitPosition({ revision: 5, positionMs: 2000 });
    expect(player.positionMs).toBe(2000);
  });

  it('keeps and clears errors', async () => {
    const { bridge, player } = await connected();
    bridge.failCommands = 'no queue entry has the key 9';

    await player.moveQueueEntry(9, null);
    expect(player.error).toBe('no queue entry has the key 9');

    bridge.failCommands = null;
    await player.togglePlay();
    expect(player.error).toBeNull();
  });

  it('stops listening', async () => {
    const { bridge, disconnect } = await connected();
    expect(bridge.playerListeners).toBe(2);

    disconnect();

    expect(bridge.playerListeners).toBe(0);
  });

  it('never polls', async () => {
    vi.useFakeTimers();
    const { bridge } = await connected();

    await vi.advanceTimersByTimeAsync(60_000);

    expect(bridge.calls.filter((call) => call === 'getPlayerState')).toHaveLength(1);
  });
});
