<script lang="ts">
  import BooksIcon from 'phosphor-svelte/lib/BooksIcon';
  import EmptyState from '../lib/components/EmptyState.svelte';
</script>

<div class="page">
  <h1 class="page-title">Library</h1>
  <EmptyState Icon={BooksIcon} title="Your library is empty">
    Your liked songs, playlists and imports from your services will live here.
  </EmptyState>
</div>
