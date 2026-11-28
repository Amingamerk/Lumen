import { describe, expect, it } from 'vitest';
import { fakeBridge, tracks } from '../testing';
import { LibraryStore } from './library.svelte';

function setUp() {
  const bridge = fakeBridge();
  return { bridge, library: new LibraryStore(bridge) };
}

describe('LibraryStore', () => {
  it('knows nothing before it connects', () => {
    const { bridge, library } = setUp();

    expect(library.summary).toBeNull();
    expect(library.isLiked('t1')).toBe(false);
    expect(bridge.calls).toEqual([]);
  });

  it('listens first, then takes the summary Rust has', async () => {
    const { bridge, library } = setUp();
    bridge.likedIds = ['t1', 't2'];

    await library.connect();

    expect(bridge.calls).toEqual(['onLikesChanged', 'onHistoryChanged', 'getLibrarySummary']);
    expect(library.summary).toEqual({ revision: 0, likedCount: 2, playedCount: 0, problem: null });
  });

  it('asks only about songs it has not heard of, and keeps the liked ones', async () => {
    const { bridge, library } = setUp();
    bridge.likedIds = ['t2'];
    await library.connect();

    await library.watch(['t1', 't2']);
    await library.watch(['t1', 't2']);

    expect(bridge.likedAsked).toEqual([['t1', 't2']]);
    expect(library.isLiked('t2')).toBe(true);
    expect(library.isLiked('t1')).toBe(false);
  });

  it('likes through Rust and shows the like when the event arrives', async () => {
    const { bridge, library } = setUp();
    await library.connect();
    const before = library.revision;

    await library.toggle('t1');

    expect(bridge.commands).toEqual([['likeTrack', 't1']]);
    expect(library.isLiked('t1')).toBe(true);
    expect(library.summary?.likedCount).toBe(1);
    expect(library.revision).toBeGreaterThan(before);
  });

  it('unlikes a liked song', async () => {
    const { bridge, library } = setUp();
    await library.connect();
    await library.toggle('t1');

    await library.toggle('t1');

    expect(bridge.commands.at(-1)).toEqual(['unlikeTrack', 't1']);
    expect(library.isLiked('t1')).toBe(false);
    expect(library.summary?.likedCount).toBe(0);
  });

  it('shows what another view changed', async () => {
    const { bridge, library } = setUp();
    await library.connect();

    bridge.emitLikes({ revision: 1, trackId: 't9', liked: true, likedCount: 1 });

    expect(library.isLiked('t9')).toBe(true);
  });

  it('drops an event older than one it has seen', async () => {
    const { bridge, library } = setUp();
    await library.connect();
    bridge.emitLikes({ revision: 4, trackId: 't1', liked: false, likedCount: 0 });

    bridge.emitLikes({ revision: 3, trackId: 't1', liked: true, likedCount: 1 });

    expect(library.isLiked('t1')).toBe(false);
    expect(library.summary?.likedCount).toBe(0);
  });

  it('counts the songs played from the history event', async () => {
    const { bridge, library } = setUp();
    await library.connect();

    bridge.emitHistory({ revision: 1, playedCount: 4 });

    expect(library.summary?.playedCount).toBe(4);
  });

  it('shows why the library cannot be used', async () => {
    const { bridge, library } = setUp();
    bridge.problem = 'the library was made by a newer Lumen';

    await library.connect();

    expect(library.summary?.problem).toBe('the library was made by a newer Lumen');
  });

  it('plays a list of the library through Rust, from one of its rows', async () => {
    const { bridge, library } = setUp();

    await library.play({ kind: 'recent' }, 4, tracks()[0].id);

    expect(bridge.commands).toEqual([['playList', { kind: 'recent' }, 4, 't1']]);
  });

  it('says why a command was refused', async () => {
    const { bridge, library } = setUp();
    bridge.failCommands = 'the library was made by a newer Lumen';

    await library.toggle('t1');

    expect(library.error).toBe('the library was made by a newer Lumen');
  });

  it('stops listening when told to', async () => {
    const { bridge, library } = setUp();

    const stop = await library.connect();
    stop();

    expect(bridge.libraryListeners).toBe(0);
  });
});
