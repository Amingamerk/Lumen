<!-- One horizontal row of cards. Buttons step through it when the cards do not all fit. -->
<script lang="ts">
  import CaretLeftIcon from 'phosphor-svelte/lib/CaretLeftIcon';
  import CaretRightIcon from 'phosphor-svelte/lib/CaretRightIcon';
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    /** The cards, each in an `<li>`. */
    children: Snippet;
    /** Where "Show all" goes. Without it there is no link. */
    onall?: () => void;
  }

  let { title, children, onall }: Props = $props();

  const uid = $props.id();
  let row: HTMLElement | undefined;
  /** Whether the row overflows, and which way it can still move. */
  let room = $state.raw({ overflowing: false, back: false, forward: false });

  function measure(element: HTMLElement) {
    const end = element.scrollWidth - element.clientWidth;
    // A pixel of slack for fractional scroll positions.
    room = {
      overflowing: end > 1,
      back: element.scrollLeft > 1,
      forward: element.scrollLeft < end - 1,
    };
  }

  /** Measures again when the row scrolls, is resized or gets other cards. */
  function watch(element: HTMLElement) {
    row = element;
    const update = () => measure(element);
    // jsdom has no ResizeObserver.
    const resizes = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(update);
    resizes?.observe(element);
    const cards = new MutationObserver(update);
    cards.observe(element, { childList: true });
    element.addEventListener('scroll', update, { passive: true });
    update();

    return () => {
      resizes?.disconnect();
      cards.disconnect();
      element.removeEventListener('scroll', update);
    };
  }

  /** Moves one row width. The stylesheet decides whether it glides. */
  function step(direction: 1 | -1) {
    if (row && (direction === 1 ? room.forward : room.back)) {
      row.scrollBy({ left: direction * row.clientWidth });
    }
  }
</script>

<section class="shelf" aria-labelledby="{uid}-title">
  <div class="shelf-head">
    <h2 id="{uid}-title" class="shelf-title">{title}</h2>
    <div class="flex items-center gap-3">
      {#if onall}
        <button type="button" class="show-all" onclick={onall}>
          Show all<span class="sr-only"> {title}</span>
        </button>
      {/if}
      {#if room.overflowing}
      <div class="flex gap-2">
        <button
          type="button"
          class="step"
          aria-label="Scroll {title} back"
          aria-disabled={!room.back}
          onclick={() => step(-1)}
        >
          <CaretLeftIcon size={16} weight="bold" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="step"
          aria-label="Scroll {title} forward"
          aria-disabled={!room.forward}
          onclick={() => step(1)}
        >
          <CaretRightIcon size={16} weight="bold" aria-hidden="true" />
        </button>
      </div>
      {/if}
    </div>
  </div>

  <ul class="shelf-row" {@attach watch}>
    {@render children()}
  </ul>
</section>
