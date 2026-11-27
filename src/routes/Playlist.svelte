<!-- The page behind a playlist route, which albums use too. -->
<script lang="ts">
  import MusicNotesIcon from 'phosphor-svelte/lib/MusicNotesIcon';
  import { onMount } from 'svelte';
  import { placeholderArt } from '../lib/art';
  import CoverArt from '../lib/components/CoverArt.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import SourceBadge from '../lib/components/SourceBadge.svelte';
  import TrackTable from '../lib/components/TrackTable.svelte';
  import { countLabel, formatTotalDuration } from '../lib/format';
  import type { SourceKind } from '../lib/ipc';
  import { SOURCE_NAMES } from '../lib/sources';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';

  let { catalog, id }: { catalog: CatalogStore; id: string } = $props();

  /** The store may still hold the page visited before this one. */
  const collection = $derived(catalog.collection?.id === id ? catalog.collection : null);
  const tracks = $derived(catalog.tracksOf(id));

  const comingTracks: Record<SourceKind, string> = {
    youtube: `Tracks will appear once Lumen can read your ${SOURCE_NAMES.youtube} library.`,
    soundcloud: 'Tracks will appear once Lumen can read your SoundCloud library.',
    spotify: `Lumen will list these tracks from Spotify and play their matches from ${SOURCE_NAMES.youtube}.`,
  };

  onMount(() => void catalog.loadCollection(id));
</script>

{#if collection}
  <div class="page">
    <!-- The backdrop takes the dominant colour of the cover. -->
    <header class="hero" data-tone={placeholderArt(collection.title).tone}>
      <div class="cover">
        <CoverArt title={collection.title} />
      </div>
      <div class="min-w-0">
        <h1 class="page-title playlist-title">{collection.title}</h1>
        <p class="text-ink mt-3 text-[15px] break-words">
          {collection.kind === 'album' ? `Album by ${collection.subtitle}` : collection.subtitle}
        </p>
        <p class="text-ink-soft mt-2 flex flex-wrap items-center gap-x-5 gap-y-1 tabular-nums">
          <span class="flex items-center gap-2 whitespace-nowrap">
            <SourceBadge source={collection.source} badge={collection.badge} />
            <span>From <span translate="no">{SOURCE_NAMES[collection.source]}</span></span>
          </span>
          <span class="whitespace-nowrap">
            {countLabel(collection.trackCount, 'track', 'tracks')}
          </span>
          {#if collection.trackCount > 0}
            <span class="whitespace-nowrap">{formatTotalDuration(collection.durationMs)}</span>
          {/if}
        </p>
      </div>
    </header>

    {#if collection.trackCount === 0}
      <EmptyState Icon={MusicNotesIcon} title="No tracks here yet">
        {comingTracks[collection.source]}
      </EmptyState>
    {:else}
      <section aria-labelledby="playlist-tracks">
        <h2 id="playlist-tracks" class="sr-only">Tracks</h2>
        <TrackTable {tracks} labelledby="playlist-tracks" />
      </section>
    {/if}
  </div>
{/if}
