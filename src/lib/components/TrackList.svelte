<!-- Rows are not selectable yet: playback arrives with the player. -->
<script lang="ts">
  import { formatDuration } from '../format';
  import type { TrackRow } from '../ipc';
  import CoverArt from './CoverArt.svelte';
  import SourceBadge from './SourceBadge.svelte';

  interface Props {
    tracks: TrackRow[];
    /** The id of the heading that names the list. */
    labelledby: string;
  }

  let { tracks, labelledby }: Props = $props();
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
      <p class="text-ink-faint text-right text-[13px] tabular-nums">
        {formatDuration(track.durationMs)}
      </p>
    </li>
  {/each}
</ul>
