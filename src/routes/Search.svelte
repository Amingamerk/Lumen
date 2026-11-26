<script lang="ts">
  import MagnifyingGlassIcon from 'phosphor-svelte/lib/MagnifyingGlassIcon';
  import EmptyState from '../lib/components/EmptyState.svelte';
</script>

<div class="page">
  <h1 class="page-title">Search</h1>
  <EmptyState Icon={MagnifyingGlassIcon} title="Search is not available yet">
    One box will cover <span translate="no">YouTube&nbsp;Music</span> and
    <span translate="no">SoundCloud</span> together.
  </EmptyState>
</div>
