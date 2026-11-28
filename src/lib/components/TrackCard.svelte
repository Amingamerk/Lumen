<script lang="ts">
  import type { TrackRow } from '../ipc';
  import CoverArt from './CoverArt.svelte';

  interface Props {
    track: TrackRow;
    onplay: () => void;
  }

  let { track, onplay }: Props = $props();
</script>

<button
  type="button"
  class="card w-full text-left"
  title={track.title}
  aria-label="Play {track.title}, {track.artists.join(', ')}"
  onclick={onplay}
>
  <CoverArt title={track.title} trackId={track.hasArt ? track.id : null} size={300} />
  <span class="card-title">{track.title}</span>
  <span class="card-meta">
    <span class="min-w-0 truncate">{track.artists.join(', ')}</span>
  </span>
</button>
