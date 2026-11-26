<script lang="ts">
  import VinylRecordIcon from 'phosphor-svelte/lib/VinylRecordIcon';
  import EmptyState from '../lib/components/EmptyState.svelte';
</script>

<div class="page">
  <h1 class="page-title">Home</h1>
  <EmptyState Icon={VinylRecordIcon} title="Nothing to play yet">
    Your recent tracks and picks will gather here once you start listening.
  </EmptyState>
</div>
