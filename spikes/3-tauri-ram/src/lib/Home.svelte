<script lang="ts">
  const shelves = ['Liked songs', 'Recently played', 'Rediscover', 'Radio'];
</script>

<section class="p-6">
  <h1 class="mb-4 text-2xl font-semibold">Home</h1>
  <ul class="grid grid-cols-4 gap-4">
    {#each shelves as shelf (shelf)}
      <li class="rounded-lg bg-neutral-900 p-4">
        <div class="mb-3 aspect-square rounded-md bg-neutral-800"></div>
        <p>{shelf}</p>
      </li>
    {/each}
  </ul>
</section>
