<script lang="ts">
  import type { PlaylistSummary } from '../ipc';
  import { SOURCE_NAMES } from '../sources';

  let { playlist }: { playlist: PlaylistSummary } = $props();
</script>

<span
  class="badge"
  data-source={playlist.source}
  title={SOURCE_NAMES[playlist.source]}
  translate="no"
>
  {playlist.badge}
</span>
