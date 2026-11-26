<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { Route } from '../ipc';

  interface Props {
    view: Route['view'] | undefined;
    /** Changes with every destination, so each one starts scrolled to the top. */
    pageKey: string | undefined;
    children: Snippet;
  }

  let { view, pageKey, children }: Props = $props();
</script>

<main id="main" class="stage surface" tabindex="-1" data-view={view}>
  <!-- The light the app is named after. It moves to show that the view changed. -->
  <div class="beam" aria-hidden="true"></div>
  {#key pageKey}
    <div class="scroller relative h-full px-10 pt-14 pb-10">
      {@render children()}
    </div>
  {/key}
</main>
