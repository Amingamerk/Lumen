import { afterEach, describe, expect, it, vi } from 'vitest';
import type { NowPlaying } from '../ipc';
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

  it('gives notice of a refused command until one succeeds', async () => {
    const { bridge, player } = await connected();
    bridge.failCommands = 'no queue entry has the key 9';

    await player.moveQueueEntry(9, null);
    expect(player.notice?.message).toBe('no queue entry has the key 9');
    expect(player.error).toBeNull();

    bridge.failCommands = null;
    await player.togglePlay();
    expect(player.notice).toBeNull();
  });

  it('gives a new notice for the same refusal again', async () => {
    const { bridge, player } = await connected();
    bridge.failCommands = 'sample track, cannot be played';
    await player.togglePlay();
    const first = player.notice;
    player.dismissNotice();
    expect(player.notice).toBeNull();

    await player.togglePlay();

    expect(player.notice?.message).toBe('sample track, cannot be played');
    expect(player.notice).not.toBe(first);
  });

  it('gives notice of a playback failure once, which a command leaves up', async () => {
    const { bridge, player } = await connected();
    const failed = { status: 'error' as const, error: 'Lumen could not play “tone”.' };

    bridge.emitPlayer(playerState({ revision: 2, ...failed }));
    const first = player.notice;
    expect(first?.message).toBe(failed.error);
    await player.setVolume(10);
    bridge.emitPlayer(playerState({ revision: 3, volume: 10, ...failed }));
    expect(player.notice).toBe(first);

    bridge.emitPlayer(playerState({ revision: 4 }));
    expect(player.notice).toBeNull();
  });

  it('gives notice again when another track fails', async () => {
    const { bridge, player } = await connected();
    const state = playerState({ revision: 2, status: 'error', error: 'broken' });
    bridge.emitPlayer(state);
    player.dismissNotice();

    const current = { ...(state.current as NowPlaying), key: 2 };
    bridge.emitPlayer({ ...state, revision: 3, current });

    expect(player.notice?.message).toBe('broken');
  });

  it('gives notice once of a song that was skipped, and again for the next one', async () => {
    const { bridge, player } = await connected();
    const skipped = { key: 4, message: 'Lumen skipped “Paper”: this song is unavailable.' };

    bridge.emitPlayer(playerState({ revision: 2, skipped }));
    const first = player.notice;
    expect(first?.message).toBe(skipped.message);
    bridge.emitPlayer(playerState({ revision: 3, skipped, volume: 10 }));
    expect(player.notice).toBe(first);
    player.dismissNotice();
    bridge.emitPlayer(playerState({ revision: 4, skipped }));
    expect(player.notice).toBeNull();

    bridge.emitPlayer(playerState({ revision: 5, skipped: { ...skipped, key: 5 } }));
    expect(player.notice).not.toBeNull();
  });

  it('says a failure again when Play is pressed and it fails again', async () => {
    const { bridge, player } = await connected();
    const failed = playerState({ revision: 2, status: 'error', error: 'missing' });
    bridge.emitPlayer(failed);
    player.dismissNotice();

    await player.togglePlay();
    bridge.emitPlayer({ ...failed, revision: 3 });

    expect(player.notice?.message).toBe('missing');
  });

  it('keeps why it could not connect', async () => {
    const bridge = fakeBridge();
    bridge.getPlayerState = async () => {
      throw 'ipc unavailable';
    };

    const { player } = await connected(bridge);

    expect(player.error).toBe('ipc unavailable');
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
