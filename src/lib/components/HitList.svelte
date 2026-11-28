<!-- Albums, artists or playlists as rows, with the picture first, the name, then one line of detail. -->
<script lang="ts">
  import { hitLine } from '../format';
  import type { HitRow } from '../ipc';
  import CoverArt from './CoverArt.svelte';
  import VirtualRows from './VirtualRows.svelte';

  interface Props {
    hits: HitRow[];
    /** The id of the heading that names the list. */
    labelledby: string;
  }

  let { hits, labelledby }: Props = $props();
</script>

<VirtualRows
  items={hits}
  rowHeight={64}
  {labelledby}
  key={(hit) => hit.id}
  rowClass="hit"
  rowData={(hit) => hit.kind}
>
  {#snippet row(hit)}
    <div class="size-12">
      <CoverArt title={hit.title} trackId={hit.hasArt ? hit.id : null} />
    </div>
    <div class="min-w-0">
      <p class="truncate text-[15px] font-semibold" title={hit.title}>{hit.title}</p>
      <p class="text-ink-faint truncate text-[13px]">{hitLine(hit)}</p>
    </div>
  {/snippet}
</VirtualRows>
