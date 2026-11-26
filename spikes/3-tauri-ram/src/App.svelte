<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import Home from './lib/Home.svelte';
  import TrackList from './lib/TrackList.svelte';
  import type { View } from './lib/types';

  const nav: { view: View; label: string }[] = [
    { view: 'home', label: 'Home' },
    { view: 'list', label: 'Library' },
  ];

  // Unset until Rust says which screen to open, so --list never mounts Home first.
  let view = $state<View>();

  invoke<View>('initial_view')
    .then((initial) => (view ??= initial))
    .catch(() => {
      view ??= 'home';
    });
</script>

<div class="grid h-full grid-cols-[12rem_1fr] grid-rows-[1fr_4.5rem]">
  <nav class="flex flex-col gap-1 bg-neutral-900 p-3">
    <p class="px-3 py-2 text-lg font-semibold text-amber-400">Lumen</p>
    {#each nav as item (item.view)}
      <button
        type="button"
        class={[
          'rounded-md px-3 py-2 text-left hover:bg-neutral-800',
          view === item.view && 'bg-neutral-800 text-white',
        ]}
        aria-current={view === item.view ? 'page' : undefined}
        onclick={() => (view = item.view)}
      >
        {item.label}
      </button>
    {/each}
  </nav>

  <main class="min-h-0 overflow-hidden">
    {#if view === 'home'}
      <Home />
    {:else if view === 'list'}
      <TrackList />
    {/if}
  </main>

  <footer class="col-span-2 flex items-center border-t border-neutral-800 bg-neutral-900 px-4 text-neutral-500">
    Nothing playing
  </footer>
</div>
