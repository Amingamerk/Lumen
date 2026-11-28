<!-- Songs as rows, drawn a window at a time. A click anywhere on a row plays it. -->
<script lang="ts">
  import PlusIcon from 'phosphor-svelte/lib/PlusIcon';
  import { releasePointerFocus } from '../focus';
  import { formatDuration } from '../format';
  import type { TrackRow } from '../ipc';
  import CoverArt from './CoverArt.svelte';
  import LikeButton from './LikeButton.svelte';
  import SourceBadge from './SourceBadge.svelte';
  import VirtualRows from './VirtualRows.svelte';

  interface Props {
    tracks: TrackRow[];
    /** The id of the heading that names the list. */
    labelledby: string;
    onplay: (trackId: string) => void;
    /** Whether a song is liked. Without it, rows have no heart. */
    liked?: (trackId: string) => boolean;
    onlike?: (trackId: string) => void;
    /** Without it, rows have no button for adding to a playlist. */
    onadd?: (track: TrackRow) => void;
    /** Cover and title line only, for a small space such as the Top Result card. */
    compact?: boolean;
  }

  let { tracks, labelledby, onplay, liked, onlike, onadd, compact = false }: Props = $props();
</script>

<VirtualRows
  items={tracks}
  rowHeight={64}
  {labelledby}
  key={(track) => track.id}
  class={compact ? 'tracks tracks-compact' : 'tracks'}
  rowClass="track"
>
  {#snippet row(track)}
    {@const artists = track.artists.join(', ')}
    <!-- Tracks of one album share its cover. -->
    <div class="size-10">
      <CoverArt title={track.album ?? track.title} trackId={track.hasArt ? track.id : null} />
    </div>
    <div class="min-w-0">
      <p class="track-title truncate text-[15px] font-semibold" title={track.title}>{track.title}</p>
      <p class="track-meta text-ink-faint truncate text-[13px]">
        {[artists, track.album, track.durationMs > 0 ? formatDuration(track.durationMs) : ''].filter(Boolean).join(' · ')}
      </p>
    </div>
    <div class="track-sources flex justify-end gap-1">
      {#each track.sources as tag (tag.source)}
        <SourceBadge source={tag.source} badge={tag.badge} />
      {/each}
    </div>
    <div class="track-end">
      {#if liked && onlike}
        <LikeButton liked={liked(track.id)} title={track.title} ontoggle={() => onlike(track.id)} />
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
    </div>
    <!-- Lies over the whole row. -->
    <button
      type="button"
      class="row-play"
      aria-label="Play {track.title} by {artists}"
      onclick={(event) => {
        releasePointerFocus(event);
        onplay(track.id);
      }}
    ></button>
  {/snippet}
</VirtualRows>
