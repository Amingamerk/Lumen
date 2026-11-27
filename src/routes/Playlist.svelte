<!-- The page behind a playlist route, which albums use too. -->
<script lang="ts">
  import MusicNotesIcon from 'phosphor-svelte/lib/MusicNotesIcon';
  import { onMount } from 'svelte';
  import CoverArt from '../lib/components/CoverArt.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import SourceBadge from '../lib/components/SourceBadge.svelte';
  import type { SourceKind } from '../lib/ipc';
  import { SOURCE_NAMES } from '../lib/sources';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';

  let { catalog, id }: { catalog: CatalogStore; id: string } = $props();

  /** The store may still hold the page visited before this one. */
  const collection = $derived(catalog.collection?.id === id ? catalog.collection : null);

  const comingTracks: Record<SourceKind, string> = {
    youtube: `Tracks will appear once Lumen can read your ${SOURCE_NAMES.youtube} library.`,
    soundcloud: 'Tracks will appear once Lumen can read your SoundCloud library.',
    spotify: `Lumen will list these tracks from Spotify and play their matches from ${SOURCE_NAMES.youtube}.`,
  };

  onMount(() => void catalog.loadCollection(id));
</script>

{#if collection}
  <div class="page">
    <header class="flex items-end gap-7">
      <div class="cover">
        <CoverArt title={collection.title} />
      </div>
      <div class="min-w-0">
        <h1 class="page-title playlist-title">{collection.title}</h1>
        <p class="text-ink mt-3 text-[15px] break-words">
          {collection.kind === 'album' ? `Album by ${collection.subtitle}` : collection.subtitle}
        </p>
        <p class="text-ink-soft mt-2 flex items-center gap-2">
          <SourceBadge source={collection.source} badge={collection.badge} />
          <span>From <span translate="no">{SOURCE_NAMES[collection.source]}</span></span>
        </p>
      </div>
    </header>

    <EmptyState Icon={MusicNotesIcon} title="No tracks here yet">
      {comingTracks[collection.source]}
    </EmptyState>
  </div>
{/if}
