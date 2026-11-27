<!-- A placeholder cover: a title's initials under its own lamp. It fills its container. -->
<script lang="ts">
  import { placeholderArt } from '../art';

  let { title }: { title: string } = $props();

  const art = $derived(placeholderArt(title));
</script>

<div class="art" data-tone={art.tone} data-light={art.light} aria-hidden="true">
  <span translate="no">{art.initials}</span>
</div>
