<script lang="ts">
  import BooksIcon from 'phosphor-svelte/lib/BooksIcon';
  import HeartIcon from 'phosphor-svelte/lib/HeartIcon';
  import WarningCircleIcon from 'phosphor-svelte/lib/WarningCircleIcon';
  import { onMount } from 'svelte';
  import ClockCounterClockwiseIcon from 'phosphor-svelte/lib/ClockCounterClockwiseIcon';
  import CollectionCard from '../lib/components/CollectionCard.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import TrackTable from '../lib/components/TrackTable.svelte';
  import { countLabel } from '../lib/format';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';
  import type { LibraryStore } from '../lib/stores/library.svelte';

  interface Props {
    catalog: CatalogStore;
    library: LibraryStore;
    onopen: (id: string) => void;
  }

  let { catalog, library, onopen }: Props = $props();

  type Part = 'collections' | 'liked' | 'recent';

  const parts: { value: Part; label: string }[] = [
    { value: 'collections', label: 'Playlists and albums' },
    { value: 'liked', label: 'Liked songs' },
    { value: 'recent', label: 'Recently played' },
  ];

  let part = $state<Part>('collections');

  const summary = $derived(library.summary);
  const sections = $derived(
    catalog.library
      ? [
          { id: 'library-playlists', title: 'Playlists', items: catalog.library.playlists },
          { id: 'library-albums', title: 'Saved albums', items: catalog.library.albums },
        ].filter((section) => section.items.length > 0)
      : null,
  );

  // A new window whenever a like or a play changes what the list holds.
  const liked = $derived.by(() => {
    void library.revision;
    return library.likedWindow();
  });
  const recent = $derived.by(() => {
    void library.revision;
    return library.recentWindow();
  });

  onMount(() => void catalog.loadLibrary());
</script>

<div class="page">
  <h1 class="page-title">Library</h1>

  {#if summary?.problem}
    <div class="alert mt-8" role="alert">
      <WarningCircleIcon size={20} weight="fill" aria-hidden="true" />
      <p>
        Lumen could not open its library, so likes and history are off. It left the file as it was.
        Details: {summary.problem}
      </p>
    </div>
  {/if}

  <fieldset class="mt-8 flex flex-wrap gap-2">
    <legend class="sr-only">Show</legend>
    {#each parts as { value, label } (value)}
      <label class="chip">
        <input
          class="sr-only"
          type="radio"
          name="library-part"
          {value}
          checked={part === value}
          onchange={() => (part = value)}
        />
        <span>{label}</span>
      </label>
    {/each}
  </fieldset>

  {#if part === 'liked'}
    {#if summary?.likedCount}
      <section class="mt-6" aria-labelledby="library-liked">
        <h2 id="library-liked" class="shelf-title mb-2">
          Liked songs <span class="text-ink-faint text-[15px] font-normal">
            {countLabel(summary.likedCount, 'song', 'songs')}
          </span>
        </h2>
        <TrackTable
          tracks={liked}
          labelledby="library-liked"
          onplay={(position) => {
            const row = liked.row(position);
            if (row) void library.play(row.id);
          }}
          liked={(id) => library.isLiked(id)}
          onlike={(id) => void library.toggle(id)}
        />
      </section>
    {:else}
      <EmptyState Icon={HeartIcon} title="No liked songs yet">
        Press the heart on a song, in the results or the player bar, to keep it here.
      </EmptyState>
    {/if}
  {:else if part === 'recent'}
    {#if summary?.playedCount}
      <section class="mt-6" aria-labelledby="library-recent">
        <h2 id="library-recent" class="shelf-title mb-2">
          Recently played <span class="text-ink-faint text-[15px] font-normal">
            {countLabel(summary.playedCount, 'song', 'songs')}
          </span>
        </h2>
        <TrackTable
          tracks={recent}
          labelledby="library-recent"
          onplay={(position) => {
            const row = recent.row(position);
            if (row) void library.play(row.id);
          }}
          liked={(id) => library.isLiked(id)}
          onlike={(id) => void library.toggle(id)}
        />
      </section>
    {:else}
      <EmptyState Icon={ClockCounterClockwiseIcon} title="Nothing played yet">
        Songs you play show up here, the latest first.
      </EmptyState>
    {/if}
  {:else if sections?.length === 0}
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
