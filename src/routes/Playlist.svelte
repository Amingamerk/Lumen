<!-- The page behind a playlist route: a local playlist and its songs. -->
<script lang="ts">
  import MusicNotesIcon from 'phosphor-svelte/lib/MusicNotesIcon';
  import PencilSimpleIcon from 'phosphor-svelte/lib/PencilSimpleIcon';
  import TrashIcon from 'phosphor-svelte/lib/TrashIcon';
  import { tick } from 'svelte';
  import { placeholderArt } from '../lib/art';
  import CoverArt from '../lib/components/CoverArt.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import TrackTable from '../lib/components/TrackTable.svelte';
  import { countLabel, formatTotalDuration } from '../lib/format';
  import type { LibraryStore } from '../lib/stores/library.svelte';
  import type { PlaylistsStore } from '../lib/stores/playlists.svelte';

  interface Props {
    playlists: PlaylistsStore;
    library: LibraryStore;
    id: string;
  }

  let { playlists, library, id }: Props = $props();

  const playlist = $derived(playlists.find(id));
  /** A new window whenever the playlist's name or entries change. */
  const tracks = $derived.by(() => {
    void playlists.version(id);
    return playlists.windowOf(id);
  });

  let renaming = $state(false);
  let name = $state('');
  let box = $state<HTMLInputElement>();
  let confirming = $state(false);

  async function startRenaming() {
    name = playlist?.name ?? '';
    renaming = true;
    await tick();
    box?.select();
  }

  function finishRenaming(event: SubmitEvent) {
    event.preventDefault();
    if (name.trim()) void playlists.rename(id, name);
    renaming = false;
  }
</script>

{#if playlist}
  <div class="page">
    <!-- The backdrop takes the dominant colour of the cover. -->
    <header class="hero" data-tone={placeholderArt(playlist.name).tone}>
      <div class="cover">
        <CoverArt title={playlist.name} trackId={playlist.coverTrack} size={300} />
      </div>
      <div class="min-w-0">
        {#if renaming}
          <form class="flex flex-wrap gap-2" onsubmit={finishRenaming}>
            <input
              bind:this={box}
              bind:value={name}
              class="picker-name"
              type="text"
              name="playlist-name"
              aria-label="Playlist name"
              autocomplete="off"
              spellcheck="false"
              onkeydown={(event) => event.key === 'Escape' && (renaming = false)}
            />
            <button type="submit" class="chip" disabled={!name.trim()}>Save</button>
            <button type="button" class="chip" onclick={() => (renaming = false)}>Cancel</button>
          </form>
        {:else}
          <h1 class="page-title playlist-title">{playlist.name}</h1>
        {/if}
        <p class="text-ink-soft mt-3 flex flex-wrap items-center gap-x-5 gap-y-1 tabular-nums">
          <span class="whitespace-nowrap">Playlist</span>
          <span class="whitespace-nowrap">
            {countLabel(playlist.trackCount, 'track', 'tracks')}
          </span>
          {#if playlist.trackCount > 0}
            <span class="whitespace-nowrap">{formatTotalDuration(playlist.durationMs)}</span>
          {/if}
        </p>
        <div class="mt-4 flex flex-wrap gap-2">
          {#if !renaming}
            <button type="button" class="chip" onclick={startRenaming}>
              <PencilSimpleIcon size={16} aria-hidden="true" />
              <span class="ml-2">Rename</span>
            </button>
          {/if}
          {#if confirming}
            <button
              type="button"
              class="chip"
              aria-label="Delete the playlist {playlist.name}"
              onclick={() => void playlists.remove(id)}
            >
              Delete it
            </button>
            <button type="button" class="chip" onclick={() => (confirming = false)}>Keep it</button>
          {:else}
            <button type="button" class="chip" onclick={() => (confirming = true)}>
              <TrashIcon size={16} aria-hidden="true" />
              <span class="ml-2">Delete playlist</span>
            </button>
          {/if}
        </div>
      </div>
    </header>

    {#if playlist.trackCount === 0}
      <EmptyState Icon={MusicNotesIcon} title="No songs here yet">
        Press the plus on a song in the search results or your library to put it in this playlist.
      </EmptyState>
    {:else}
      <section aria-labelledby="playlist-tracks">
        <h2 id="playlist-tracks" class="sr-only">Songs</h2>
        <TrackTable
          {tracks}
          labelledby="playlist-tracks"
          onplay={(position) => {
            const row = tracks.row(position);
            if (row) void library.play({ kind: 'playlist', id }, position, row.id);
          }}
          onadd={(row) => playlists.choose({ trackId: row.id, title: row.title })}
          onremove={(position, row) => void playlists.removeEntry(id, position, row.id)}
        />
      </section>
    {/if}
  </div>
{:else if playlists.playlists}
  <div class="page">
    <EmptyState Icon={MusicNotesIcon} title="This playlist is gone">
      It was deleted. Pick another one from the sidebar.
    </EmptyState>
  </div>
{/if}
