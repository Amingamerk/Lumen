<!--
  A cover: a title's initials under its own lamp, with the real picture over them
  once it has loaded. Without a picture, or when it fails, the initials stay. It
  fills its container.
-->
<script lang="ts">
  import { type ArtSize, artUrl, placeholderArt } from '../art';

  interface Props {
    title: string;
    /** The track whose picture to show. Without it only the placeholder shows. */
    trackId?: string | null;
    /** The size to ask for: 64 for rows and the player bar, 300 for cards and page heads. */
    size?: ArtSize;
  }

  let { title, trackId = null, size = 64 }: Props = $props();

  const art = $derived(placeholderArt(title));
  const src = $derived(trackId ? artUrl(trackId, size) : null);
  /** The address that failed, so another track's picture still gets its try. */
  let failed = $state<string | null>(null);
  let loaded = $state<string | null>(null);
</script>

<div class="art" data-tone={art.tone} data-light={art.light} aria-hidden="true">
  <span translate="no">{art.initials}</span>
  {#if src && failed !== src}
    <!-- The webview loads it only when it nears the screen. -->
    <img
      {src}
      alt=""
      class="art-picture"
      data-loaded={loaded === src ? '' : undefined}
      loading="lazy"
      decoding="async"
      draggable="false"
      onload={() => (loaded = src)}
      onerror={() => (failed = src)}
    />
  {/if}
</div>
