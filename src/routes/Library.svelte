<script lang="ts">
  import BooksIcon from 'phosphor-svelte/lib/BooksIcon';
  import { onMount } from 'svelte';
  import CollectionCard from '../lib/components/CollectionCard.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';

  interface Props {
    catalog: CatalogStore;
    onopen: (id: string) => void;
  }

  let { catalog, onopen }: Props = $props();

  const sections = $derived(
    catalog.library
      ? [
          { id: 'library-playlists', title: 'Playlists', items: catalog.library.playlists },
          { id: 'library-albums', title: 'Saved albums', items: catalog.library.albums },
        ].filter((section) => section.items.length > 0)
      : null,
  );

  onMount(() => void catalog.loadLibrary());
</script>

<div class="page">
  <h1 class="page-title">Library</h1>
  {#if sections?.length === 0}
    <EmptyState Icon={BooksIcon} title="Your library is empty">
      Your liked songs, playlists and imports from your services will live here.
    </EmptyState>
  {:else if sections}
    <div class="mt-9 flex flex-col gap-9">
      {#each sections as { id, title, items } (id)}
        <section aria-labelledby={id}>
          <h2 {id} class="shelf-title mb-2">{title}</h2>
          <ul class="card-grid">
            {#each items as collection (collection.id)}
              <li class="min-w-0">
                <CollectionCard {collection} {onopen} />
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    </div>
  {/if}
</div>
