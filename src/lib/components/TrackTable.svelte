<!--
  A Collection's Tracks, however many there are: only the rows near the view are in
  the DOM. It scrolls with the page, in the nearest `[data-scroller]`.
-->
<script lang="ts">
  import type { Attachment } from 'svelte/attachments';
  import { formatDuration, formatNumber } from '../format';
  import type { TrackWindow } from '../stores/trackWindow.svelte';
  import { rowRange } from '../virtual';
  import SourceBadge from './SourceBadge.svelte';

  interface Props {
    tracks: TrackWindow;
    /** The id of the heading that names the list. */
    labelledby: string;
  }

  let { tracks, labelledby }: Props = $props();

  const ROW_HEIGHT = 56;
  const OVERSCAN = 8;

  let scrolled = $state.raw(0);
  let viewport = $state.raw(0);
  /** Whether the column names have reached the top, with rows passing under them. */
  let stuck = $state.raw(false);
  let head = $state<HTMLElement>();

  const total = $derived(tracks.total ?? 0);
  const range = $derived(
    rowRange({ scrolled, viewport, rowHeight: ROW_HEIGHT, total, overscan: OVERSCAN }),
  );
  const positions = $derived(
    Array.from({ length: range.end - range.start }, (_, at) => range.start + at),
  );

  $effect(() => tracks.show(range.start, range.end));

  /** Follows how much of the list is in view. */
  const follow: Attachment<HTMLElement> = (list) => {
    const scroller = list.closest<HTMLElement>('[data-scroller]');
    if (!scroller) return;

    const measure = () => {
      const { top } = scroller.getBoundingClientRect();
      scrolled = top - list.getBoundingClientRect().top;
      viewport = scroller.clientHeight;
      stuck = head !== undefined && head.getBoundingClientRect().top - top < 1;
    };
    scroller.addEventListener('scroll', measure, { passive: true });
    // Also measures once as soon as it starts watching.
    const resizes = new ResizeObserver(measure);
    resizes.observe(scroller);

    return () => {
      scroller.removeEventListener('scroll', measure);
      resizes.disconnect();
    };
  };
</script>

<div
  class="track-table"
  role="table"
  aria-labelledby={labelledby}
  aria-rowcount={tracks.total === null ? -1 : total + 1}
>
  <div class="track-head" role="rowgroup" data-stuck={stuck} bind:this={head}>
    <div class="track-line" role="row" aria-rowindex="1">
      <span class="track-number" role="columnheader">#</span>
      <span role="columnheader">Title</span>
      <span class="track-album" role="columnheader">Album</span>
      <span class="text-right" role="columnheader">Source</span>
      <span class="text-right" role="columnheader">Time</span>
    </div>
  </div>

  <!-- As tall as the whole list, so the scrollbar is true. Rows are placed in it. -->
  <div class="track-body" role="rowgroup" style:height="{total * ROW_HEIGHT}px" {@attach follow}>
    {#each positions as position (position)}
      {@const track = tracks.row(position)}
      <div
        class="track-line track-row"
        role="row"
        aria-rowindex={position + 2}
        style:height="{ROW_HEIGHT}px"
        style:transform="translateY({position * ROW_HEIGHT}px)"
      >
        <span class="track-number" role="cell">{formatNumber(position + 1)}</span>
        {#if track}
          {@const artists = track.artists.join(', ')}
          <span class="min-w-0" role="cell">
            <span class="block truncate text-[15px] font-semibold" title={track.title}>
              {track.title}
            </span>
            <span class="text-ink-faint block truncate text-[13px]" title={artists}>{artists}</span>
          </span>
          <span class="track-album text-ink-soft truncate" role="cell" title={track.album}>
            {track.album}
          </span>
          <span class="flex justify-end gap-1" role="cell">
            {#each track.sources as tag (tag.source)}
              <SourceBadge source={tag.source} badge={tag.badge} />
            {/each}
          </span>
          <span class="text-ink-faint text-right text-[13px] tabular-nums" role="cell">
            {formatDuration(track.durationMs)}
          </span>
        {:else}
          <span role="cell">
            <span class="track-blank" aria-hidden="true"></span>
            <span class="sr-only">Loading…</span>
          </span>
          <span class="track-album" role="cell"></span>
          <span role="cell"></span>
          <span role="cell"></span>
        {/if}
      </div>
    {/each}
  </div>
</div>
