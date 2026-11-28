<script lang="ts">
  import VinylRecordIcon from 'phosphor-svelte/lib/VinylRecordIcon';
  import { onMount } from 'svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import PlaylistCard from '../lib/components/PlaylistCard.svelte';
  import Shelf from '../lib/components/Shelf.svelte';
  import TrackCard from '../lib/components/TrackCard.svelte';
  import type { PlayList } from '../lib/ipc';
  import type { CatalogStore } from '../lib/stores/catalog.svelte';
  import type { LibraryPart, LibraryStore } from '../lib/stores/library.svelte';

  interface Props {
    catalog: CatalogStore;
    library: LibraryStore;
    onopen: (id: string) => void;
    /** Opens the Library on this tab. */
    onshowall: (part: LibraryPart) => void;
  }

  let { catalog, library, onopen, onshowall }: Props = $props();

  onMount(() => void catalog.loadHome());

  const home = $derived(catalog.home);
  const empty = $derived(
    home !== null && !home.recent.length && !home.liked.length && !home.playlists.length,
  );

  const songShelves = $derived(
    home
      ? ([
          { title: 'Recently played', cards: home.recent, list: { kind: 'recent' }, part: 'recent' },
          { title: 'Liked songs', cards: home.liked, list: { kind: 'liked' }, part: 'liked' },
        ] as const satisfies readonly {
          title: string;
          cards: unknown;
          list: PlayList;
          part: LibraryPart;
        }[])
      : [],
  );
</script>

<div class="page">
  <h1 class="page-title">Home</h1>
  {#if empty}
    <EmptyState Icon={VinylRecordIcon} title="Nothing to play yet">
      Search for a song and play it. Your recent songs, likes and playlists will gather here.
    </EmptyState>
  {:else if home}
    <div class="mt-9 flex flex-col gap-8">
      {#each songShelves as { title, cards, list, part } (title)}
        {#if cards.length}
          <Shelf {title} onall={() => onshowall(part)}>
            {#each cards as track, position (track.id)}
              <li class="min-w-0">
                <TrackCard {track} onplay={() => void library.play(list, position, track.id)} />
              </li>
            {/each}
          </Shelf>
        {/if}
      {/each}
      {#if home.playlists.length}
        <Shelf title="Your playlists" onall={() => onshowall('collections')}>
          {#each home.playlists as playlist (playlist.id)}
            <li class="min-w-0">
              <PlaylistCard {playlist} {onopen} />
            </li>
          {/each}
        </Shelf>
      {/if}
    </div>
  {/if}
</div>
