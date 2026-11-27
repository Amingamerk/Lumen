<script lang="ts">
  import VinylRecordIcon from 'phosphor-svelte/lib/VinylRecordIcon';
  import { onMount } from 'svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import Shelf from '../lib/components/Shelf.svelte';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';

  interface Props {
    catalog: CatalogStore;
    onopen: (id: string) => void;
  }

  let { catalog, onopen }: Props = $props();

  onMount(() => void catalog.loadHome());
</script>

<div class="page">
  <h1 class="page-title">Home</h1>
  {#if catalog.home?.length === 0}
    <EmptyState Icon={VinylRecordIcon} title="Nothing to play yet">
      Your recent tracks and picks will gather here once you start listening.
    </EmptyState>
  {:else if catalog.home}
    <div class="mt-9 flex flex-col gap-8">
      {#each catalog.home as shelf (shelf.id)}
        <Shelf title={shelf.title} items={shelf.items} {onopen} />
      {/each}
    </div>
  {/if}
</div>
