import { describe, expect, it } from 'vitest';
import { fakeBridge } from '../testing';
import { PlaylistsStore } from './playlists.svelte';

function setUp() {
  const bridge = fakeBridge();
  return { bridge, playlists: new PlaylistsStore(bridge) };
}

describe('PlaylistsStore', () => {
  it('knows nothing before Rust has said, then mirrors the list', async () => {
    const { bridge, playlists } = setUp();
    expect(playlists.playlists).toBeNull();

    await playlists.connect();

    expect(playlists.playlists?.map((p) => p.name)).toEqual(['Night shift']);
    expect(bridge.calls).toEqual(['onPlaylistsChanged', 'getPlaylists']);
  });

  it('follows the events that answer a change', async () => {
    const { playlists } = setUp();
    await playlists.connect();

    const made = await playlists.create('  Tunnel mixes ');
    await playlists.add(made?.id ?? '', 't1');

    expect(playlists.playlists?.map((p) => [p.name, p.trackCount])).toEqual([
      ['Night shift', 2],
      ['Tunnel mixes', 1],
    ]);
    expect(playlists.version(made?.id ?? '')).toBeGreaterThan(0);
    expect(playlists.version('night-shift')).toBe(0);
  });

  it('drops an event older than one it has', async () => {
    const { bridge, playlists } = setUp();
    await playlists.connect();
    bridge.emitPlaylists({ revision: 9, playlists: [], changed: 'night-shift' });

    bridge.emitPlaylists({
      revision: 4,
      playlists: [{ id: 'old', name: 'Old', trackCount: 0, durationMs: 0, coverTrack: null }],
      changed: 'old',
    });

    expect(playlists.playlists).toEqual([]);
    expect(playlists.version('old')).toBe(0);
  });

  it('renames, removes entries and deletes through Rust', async () => {
    const { bridge, playlists } = setUp();
    await playlists.connect();

    await playlists.rename('night-shift', 'Late');
    await playlists.removeEntry('night-shift', 0, 'n1');
    await playlists.remove('night-shift');

    expect(bridge.commands).toEqual([
      ['renamePlaylist', 'night-shift', 'Late'],
      ['removeFromPlaylist', 'night-shift', 0, 'n1'],
      ['deletePlaylist', 'night-shift'],
    ]);
    expect(playlists.playlists).toEqual([]);
  });

  it('says why Rust refused, and keeps what it has', async () => {
    const { playlists } = setUp();
    await playlists.connect();

    const made = await playlists.create('   ');

    expect(made).toBeNull();
    expect(playlists.error).toBe('a playlist needs a name');
    expect(playlists.playlists).toHaveLength(1);
    await playlists.rename('night-shift', 'Fine');
    expect(playlists.error).toBeNull();
  });

  it('holds the song being chosen for until the choice is over', () => {
    const { playlists } = setUp();

    playlists.choose({ trackId: 't1', title: 'Harbor Lights' });
    expect(playlists.picking).toEqual({ trackId: 't1', title: 'Harbor Lights' });
    playlists.stopChoosing();

    expect(playlists.picking).toBeNull();
  });

  it('stops listening', async () => {
    const { bridge, playlists } = setUp();

    const stop = await playlists.connect();
    stop();

    expect(bridge.playlistListeners).toBe(0);
  });
});
