<!-- The player's frame. Controls stay disabled until playback exists (Milestone 2). -->
<script lang="ts">
  import MusicNoteIcon from 'phosphor-svelte/lib/MusicNoteIcon';
  import PlayIcon from 'phosphor-svelte/lib/PlayIcon';
  import SkipBackIcon from 'phosphor-svelte/lib/SkipBackIcon';
  import SkipForwardIcon from 'phosphor-svelte/lib/SkipForwardIcon';
</script>

<section class="player surface" aria-label="Player">
  <div class="flex min-w-0 items-center gap-3">
    <div class="art-slot" aria-hidden="true">
      <MusicNoteIcon size={22} />
    </div>
    <p class="text-ink-soft min-w-0 truncate">Nothing is playing</p>
  </div>

  <div class="flex flex-col items-center gap-2">
    <div class="flex items-center gap-4">
      <button type="button" class="transport" disabled aria-label="Previous">
        <SkipBackIcon size={20} weight="fill" aria-hidden="true" />
      </button>
      <button type="button" class="transport transport-play" disabled aria-label="Play">
        <PlayIcon size={18} weight="fill" aria-hidden="true" />
      </button>
      <button type="button" class="transport" disabled aria-label="Next">
        <SkipForwardIcon size={20} weight="fill" aria-hidden="true" />
      </button>
    </div>
    <div class="seek" aria-hidden="true">
      <span>0:00</span>
      <span class="rail"></span>
      <span>0:00</span>
    </div>
  </div>
</section>
