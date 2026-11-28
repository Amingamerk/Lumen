<script lang="ts">
  import { countLabel } from '../format';
  import type { PlaylistSummary } from '../ipc';
  import CoverArt from './CoverArt.svelte';

  interface Props {
    playlist: PlaylistSummary;
    onopen: (id: string) => void;
  }

  let { playlist, onopen }: Props = $props();

  /** Rust owns the route, so the link never changes the page by itself. */
  function open(event: MouseEvent) {
    event.preventDefault();
    onopen(playlist.id);
  }
</script>

<a
  class="card"
  href="#playlist/{playlist.id}"
  draggable="false"
  title={playlist.name}
  aria-label="{playlist.name}, playlist, {countLabel(playlist.trackCount, 'track', 'tracks')}"
  onclick={open}
>
  <CoverArt title={playlist.name} />
  <span class="card-title">{playlist.name}</span>
  <span class="card-meta">
    <span class="min-w-0 truncate">{countLabel(playlist.trackCount, 'track', 'tracks')}</span>
  </span>
</a>
