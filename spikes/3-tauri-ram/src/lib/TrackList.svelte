<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { Track, TrackSlice } from './types';

  const ROW_HEIGHT = 44;
  // Extra rows above and below the viewport, so fast scrolling does not show gaps.
  const OVERSCAN = 5;

  let scrollTop = $state(0);
  let viewportHeight = $state(0);
  let total = $state(0);
  // Only the rows on screen. The other tracks stay in Rust.
  let shown = $state.raw<{ offset: number; tracks: Track[] }>({ offset: 0, tracks: [] });
  let error = $state('');

  const offset = $derived(Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN));
  const limit = $derived(Math.ceil(viewportHeight / ROW_HEIGHT) + 2 * OVERSCAN);

  let latestRequest = 0;

  async function load(offset: number, limit: number) {
    const request = ++latestRequest;
    try {
      const slice = await invoke<TrackSlice>('get_track_slice', { offset, limit });
      // A newer scroll position has been asked for since; drop this answer.
      if (request !== latestRequest) return;
      total = slice.total;
      shown = { offset, tracks: slice.tracks };
      error = '';
    } catch (cause) {
      // A stale failure must not put an error over a newer slice.
      if (request !== latestRequest) return;
      error = String(cause);
    }
  }

  // Runs on mount and when the window of visible rows moves, not on every scrolled pixel.
  $effect(() => {
    if (viewportHeight > 0) load(offset, limit);
  });

  function duration(ms: number) {
    const seconds = Math.floor(ms / 1000);
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
  }
</script>

<section class="flex h-full flex-col">
  <header class="px-6 pt-6 pb-3">
    <h1 class="text-2xl font-semibold">Library</h1>
    <p class="text-neutral-500">{total} tracks</p>
    {#if error}
      <p role="alert" class="text-red-400">{error}</p>
    {/if}
  </header>

  <div
    class="min-h-0 flex-1 overflow-y-auto"
    bind:clientHeight={viewportHeight}
    onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
  >
    <div class="relative overflow-hidden" style:height="{total * ROW_HEIGHT}px">
      <ul style:transform="translateY({shown.offset * ROW_HEIGHT}px)">
        {#each shown.tracks as track (track.id)}
          <li
            class="grid grid-cols-[4rem_2fr_1fr_1fr_4rem] items-center gap-3 px-6 hover:bg-neutral-900"
            style:height="{ROW_HEIGHT}px"
          >
            <span class="text-neutral-500">{track.id}</span>
            <span class="truncate">{track.title}</span>
            <span class="truncate text-neutral-400">{track.artist}</span>
            <span class="truncate text-neutral-400">{track.album}</span>
            <span class="text-right text-neutral-500">{duration(track.durationMs)}</span>
          </li>
        {/each}
      </ul>
    </div>
  </div>
</section>
