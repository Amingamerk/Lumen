<script lang="ts">
  import MusicNotesIcon from 'phosphor-svelte/lib/MusicNotesIcon';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import SourceBadge from '../lib/components/SourceBadge.svelte';
  import type { PlaylistSummary, SourceKind } from '../lib/ipc';
  import { SOURCE_NAMES } from '../lib/sources';

  let { playlist }: { playlist: PlaylistSummary } = $props();

  const comingTracks: Record<SourceKind, string> = {
    youtube: `Tracks will appear once Lumen can read your ${SOURCE_NAMES.youtube} library.`,
    soundcloud: 'Tracks will appear once Lumen can read your SoundCloud library.',
    spotify: `Lumen will list these tracks from Spotify and play their matches from ${SOURCE_NAMES.youtube}.`,
  };
</script>

<div class="page">
  <header class="flex items-end gap-7">
    <div class="cover" data-source={playlist.source} aria-hidden="true">
      <MusicNotesIcon size={56} weight="duotone" />
    </div>
    <div class="min-w-0">
      <h1 class="page-title playlist-title">{playlist.name}</h1>
      <p class="text-ink-soft mt-4 flex items-center gap-2">
        <SourceBadge {playlist} />
        <span>From <span translate="no">{SOURCE_NAMES[playlist.source]}</span></span>
      </p>
    </div>
  </header>

  <EmptyState Icon={MusicNotesIcon} title="No tracks here yet">
    {comingTracks[playlist.source]}
  </EmptyState>
</div>
