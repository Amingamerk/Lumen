<!-- Tracks as rows. A click anywhere on a row plays it. -->
<script lang="ts">
  import PlusIcon from 'phosphor-svelte/lib/PlusIcon';
  import { releasePointerFocus } from '../focus';
  import { formatDuration } from '../format';
  import type { TrackRow } from '../ipc';
  import CoverArt from './CoverArt.svelte';
  import LikeButton from './LikeButton.svelte';
  import SourceBadge from './SourceBadge.svelte';

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
  }

  let { tracks, labelledby, onplay, liked, onlike, onadd }: Props = $props();
</script>

<ul class="tracks" aria-labelledby={labelledby}>
  {#each tracks as track (track.id)}
    {@const artists = track.artists.join(', ')}
    <li class="track">
      <!-- Tracks of one album share its cover. -->
      <div class="size-10">
        <CoverArt title={track.album ?? track.title} />
      </div>
      <div class="min-w-0">
        <p class="truncate text-[15px] font-semibold" title={track.title}>{track.title}</p>
        <p class="text-ink-faint truncate text-[13px]" title={artists}>{artists}</p>
      </div>
      <p class="track-album text-ink-soft truncate" title={track.album}>{track.album}</p>
      <div class="flex justify-end gap-1">
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
        <p class="text-ink-faint text-right text-[13px] tabular-nums">
          {formatDuration(track.durationMs)}
        </p>
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
    </li>
  {/each}
</ul>
