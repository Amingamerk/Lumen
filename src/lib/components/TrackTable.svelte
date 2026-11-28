<!--
  A Collection's Tracks, however many there are: only the rows near the view are in
  the DOM. It scrolls with the page, in the nearest `[data-scroller]`.
-->
<script lang="ts">
  import type { Attachment } from 'svelte/attachments';
  import PlusIcon from 'phosphor-svelte/lib/PlusIcon';
  import XIcon from 'phosphor-svelte/lib/XIcon';
  import { releasePointerFocus } from '../focus';
  import { formatDuration, formatNumber } from '../format';
  import type { QueuePlacement, TrackRow } from '../ipc';
  import type { TrackWindow } from '../stores/trackWindow.svelte';
  import { rowRange } from '../virtual';
  import LikeButton from './LikeButton.svelte';
  import QueueButtons from './QueueButtons.svelte';
  import SourceBadge from './SourceBadge.svelte';

  interface Props {
    tracks: TrackWindow;
    /** The id of the heading that names the list. */
    labelledby: string;
    /** Called with a row's position in the whole list, counted from 0. */
    onplay: (position: number) => void;
    /** Whether a song is liked. Without it, rows have no heart. */
    liked?: (trackId: string) => boolean;
    onlike?: (trackId: string) => void;
    /** Without it, rows have no button for adding to a playlist. */
    onadd?: (track: TrackRow) => void;
    /** Without it, rows have no buttons for the queue. */
    onqueue?: (trackId: string, placement: QueuePlacement) => void;
    /** Called with a row's position and song. Without it, rows have no remove button. */
    onremove?: (position: number, track: TrackRow) => void;
  }

  let { tracks, labelledby, onplay, liked, onlike, onadd, onqueue, onremove }: Props = $props();

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
            <!-- Lies over the whole row. -->
            <button
              type="button"
              class="row-play"
              aria-label="Play {track.title} by {artists}"
              onclick={(event) => {
                releasePointerFocus(event);
                onplay(position);
              }}
            ></button>
          </span>
          <span class="track-album text-ink-soft truncate" role="cell" title={track.album}>
            {track.album}
          </span>
          <span class="flex justify-end gap-1" role="cell">
            {#each track.sources as tag (tag.source)}
              <SourceBadge source={tag.source} badge={tag.badge} />
            {/each}
          </span>
          <span class="track-end" role="cell">
            {#if liked && onlike}
              <LikeButton
                liked={liked(track.id)}
                title={track.title}
                ontoggle={() => onlike(track.id)}
              />
            {/if}
            {#if onqueue}
              <QueueButtons
                title={track.title}
                onqueue={(placement) => onqueue(track.id, placement)}
              />
            {/if}
            {#if onadd}
              <button
                type="button"
                class="row-action"
                aria-label="Add {track.title} to a playlist"
                onclick={(event) => {
                  releasePointerFocus(event);
                  onadd(track);
                }}
              >
                <PlusIcon size={18} aria-hidden="true" />
              </button>
            {/if}
            {#if onremove}
              <button
                type="button"
                class="row-action"
                aria-label="Remove {track.title} from this playlist"
                onclick={(event) => {
                  releasePointerFocus(event);
                  onremove(position, track);
                }}
              >
                <XIcon size={18} aria-hidden="true" />
              </button>
            {/if}
            <span class="text-ink-faint text-right text-[13px] tabular-nums">
              {track.durationMs > 0 ? formatDuration(track.durationMs) : ''}
            </span>
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
