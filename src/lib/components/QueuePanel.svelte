<!--
  The Queue: what is playing, then what comes next, or what played before. Only the
  rows near the view are drawn, and Rust is asked for just those. Rows that come next
  move by dragging their handle, or with the arrow keys on it.
-->
<script lang="ts">
  import DotsSixVerticalIcon from 'phosphor-svelte/lib/DotsSixVerticalIcon';
  import XIcon from 'phosphor-svelte/lib/XIcon';
  import { tick } from 'svelte';
  import type { Attachment } from 'svelte/attachments';
  import { releasePointerFocus } from '../focus';
  import { formatNumber } from '../format';
  import type { QueueItem } from '../ipc';
  import { dropGap, isSamePlace } from '../reorder';
  import type { PlayerStore } from '../stores/player.svelte';
  import type { QueueStore } from '../stores/queue.svelte';
  import { rowRange } from '../virtual';

  interface Props {
    player: PlayerStore;
    queue: QueueStore;
    onclose: () => void;
  }

  let { player, queue, onclose }: Props = $props();

  const ROW_HEIGHT = 56;
  const OVERSCAN = 6;
  /** How near an edge a drag starts to scroll, and how far it goes each frame. */
  const EDGE = 40;
  const STEP = 8;

  const current = $derived(player.state?.current ?? null);
  const upcoming = $derived(player.state?.upcoming ?? 0);
  const playedCount = $derived(player.state?.history ?? 0);
  const queueRevision = $derived(player.state?.queueRevision);

  let showing = $state<'next' | 'history'>('next');
  // With nothing played, there is no history to show.
  const view = $derived(showing === 'history' && playedCount > 0 ? 'history' : 'next');
  const total = $derived(view === 'history' ? playedCount : upcoming);

  let scrolled = $state.raw(0);
  let viewport = $state.raw(0);
  let list = $state<HTMLOListElement>();

  const range = $derived(
    rowRange({ scrolled, viewport, rowHeight: ROW_HEIGHT, total, overscan: OVERSCAN }),
  );
  const positions = $derived(
    Array.from({ length: range.end - range.start }, (_, at) => range.start + at),
  );
  // One row more than is drawn: a heading depends on the row before it.
  const offset = $derived(Math.max(0, range.start - 1));
  const limit = $derived(range.end - offset);

  $effect(() => {
    if (queueRevision === undefined || limit <= 0) return;
    if (view === 'history') void queue.loadPlayed(offset, limit);
    else void queue.load(offset, limit);
  });

  function itemAt(position: number): QueueItem | undefined {
    const slice = view === 'history' ? queue.played : queue.slice;
    return slice?.items[position - slice.offset];
  }

  /** Where the Recommender's picks start, and where the queue takes over again. */
  function headingAt(position: number): string | null {
    if (view === 'history') return null;
    const item = itemAt(position);
    if (!item) return null;
    const before = position === 0 ? undefined : itemAt(position - 1);
    if (position > 0 && !before) return null;
    if (item.autoplay === (before?.autoplay ?? false)) return null;
    return item.autoplay ? 'Up next from recommendations' : 'Back to your queue';
  }

  /** Follows how much of the list is in view. */
  const follow: Attachment<HTMLElement> = (scroller) => {
    const measure = () => {
      if (!list) return;
      scrolled = scroller.getBoundingClientRect().top - list.getBoundingClientRect().top;
      viewport = scroller.clientHeight;
    };
    scroller.addEventListener('scroll', measure, { passive: true });
    // Also measures once as soon as it starts watching.
    const resizes = new ResizeObserver(measure);
    resizes.observe(scroller);

    return () => {
      scroller.removeEventListener('scroll', measure);
      resizes.disconnect();
    };
  };

  interface Drag {
    key: number;
    /** Where the entry was picked up. */
    from: number;
    gap: number;
    pointerY: number;
  }

  let drag = $state.raw<Drag | null>(null);
  let frame = 0;

  function grab(event: PointerEvent, key: number, from: number) {
    if (event.button !== 0 || !list) return;
    event.preventDefault();
    // The list holds the pointer: the row may be scrolled out of the DOM.
    list.setPointerCapture?.(event.pointerId);
    drag = { key, from, gap: from, pointerY: event.clientY };
    frame = requestAnimationFrame(autoScroll);
  }

  function aim(pointerY: number) {
    if (!drag || !list) return;
    const gap = dropGap(pointerY, list.getBoundingClientRect().top, ROW_HEIGHT, total);
    drag = { ...drag, gap, pointerY };
  }

  /** Runs only during a drag, scrolling while the pointer is near an edge. */
  function autoScroll() {
    const scroller = list?.parentElement;
    if (!drag || !scroller) return;
    const { top, bottom, height } = scroller.getBoundingClientRect();
    const step = drag.pointerY < top + EDGE ? -STEP : drag.pointerY > bottom - EDGE ? STEP : 0;
    if (step && height > 0) {
      scroller.scrollTop += step;
      aim(drag.pointerY);
    }
    frame = requestAnimationFrame(autoScroll);
  }

  function abort() {
    cancelAnimationFrame(frame);
    drag = null;
  }

  function drop(event: PointerEvent) {
    aim(event.clientY);
    const held = drag;
    abort();
    if (!held) return;
    // The queue may have moved on during the drag: find the entry again.
    const now = queue.slice?.items.findIndex((item) => item.key === held.key) ?? -1;
    const from = now < 0 ? held.from : now + (queue.slice?.offset ?? 0);
    if (isSamePlace(from, held.gap)) return;
    const anchor = held.gap < total ? itemAt(held.gap) : null;
    // The row to land before has not arrived: better to do nothing than guess.
    if (anchor === undefined) return;
    void player.moveQueueEntry(held.key, anchor?.key ?? null);
  }

  $effect(() => abort);

  let announcement = $state.raw('');
  /** The entry moved by key, which takes the focus back once its new row lands. */
  let refocus: { key: number; slice: unknown } | null = null;

  async function step(event: KeyboardEvent, item: QueueItem, position: number) {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
    event.preventDefault();
    const up = event.key === 'ArrowUp';
    const target = up ? position - 1 : position + 1;
    if (target < 0 || target >= total) return;
    const gap = up ? target : position + 2;
    const anchor = gap < total ? itemAt(gap) : null;
    if (anchor === undefined) return;
    const count = total;
    refocus = { key: item.key, slice: queue.slice };
    if (!(await player.moveQueueEntry(item.key, anchor?.key ?? null))) {
      refocus = null;
      return;
    }
    announcement = `Moved ${item.track.title} to position ${formatNumber(target + 1)} of ${formatNumber(count)}`;
  }

  $effect(() => {
    const slice = queue.slice;
    if (!refocus || slice === refocus.slice) return;
    const { key } = refocus;
    refocus = null;
    void tick().then(() => list?.querySelector<HTMLElement>(`[data-key="${key}"]`)?.focus());
  });
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key !== 'Escape' || !drag) return;
    event.preventDefault();
    abort();
  }}
/>

<aside id="queue-panel" class="queue-panel surface" aria-label="Queue">
  <div class="flex items-center justify-between gap-3">
    <h2 class="shelf-title">Queue</h2>
    <button
      type="button"
      class="step"
      aria-label="Close queue"
      onclick={(event) => {
        releasePointerFocus(event);
        onclose();
      }}
    >
      <XIcon size={16} aria-hidden="true" />
    </button>
  </div>

  {#if current}
    {@const artists = current.track.artists.join(', ')}
    <section>
      <h3 class="queue-label">Now playing</h3>
      <p class="truncate text-[15px] font-semibold" title={current.track.title}>
        {current.track.title}
      </p>
      <p class="text-ink-faint truncate text-[13px]" title={artists}>{artists}</p>
    </section>
  {/if}

  <div class="flex items-center justify-between gap-3">
    <h3 class="queue-label">{view === 'history' ? 'History' : 'Next up'}</h3>
    {#if view === 'next' && upcoming > 0}
      <button
        type="button"
        class="chip"
        onclick={(event) => {
          releasePointerFocus(event);
          void player.clearQueue();
        }}
      >
        Clear queue
      </button>
    {/if}
    {#if playedCount > 0}
      <button
        type="button"
        class="chip"
        onclick={(event) => {
          releasePointerFocus(event);
          showing = view === 'history' ? 'next' : 'history';
        }}
      >
        {view === 'history' ? 'Show next up' : 'Show history'}
      </button>
    {/if}
  </div>
  {#if total === 0}
    <p class="text-ink-faint">Nothing up next</p>
  {:else}
    <div class="queue-scroll scroller" {@attach follow}>
      <ol
        bind:this={list}
        class="queue-list"
        style:height="{total * ROW_HEIGHT}px"
        onpointermove={(event) => aim(event.clientY)}
        onpointerup={drop}
        onpointercancel={abort}
      >
        {#each positions as position (position)}
          {@const item = itemAt(position)}
          {#if item}
            {@const heading = headingAt(position)}
            {@const artists = item.track.artists.join(', ')}
            <li
              class="queue-row"
              data-dragged={drag?.key === item.key}
              aria-setsize={total}
              aria-posinset={position + 1}
              style:height="{ROW_HEIGHT}px"
              style:transform="translateY({position * ROW_HEIGHT}px)"
            >
              <div class="min-w-0 flex-1">
                {#if heading}
                  <p class="queue-heading">{heading}</p>
                {/if}
                <p class="truncate font-semibold" title={item.track.title}>{item.track.title}</p>
                <p class="text-ink-faint truncate text-[12px]" title={artists}>{artists}</p>
              </div>
              {#if view === 'next'}
              <button
                type="button"
                class="queue-handle"
                aria-label="Move {item.track.title}"
                title="Drag, or press the up and down arrows"
                data-key={item.key}
                onpointerdown={(event) => grab(event, item.key, position)}
                onkeydown={(event) => void step(event, item, position)}
              >
                <DotsSixVerticalIcon size={18} weight="bold" aria-hidden="true" />
              </button>
              <button
                type="button"
                class="queue-handle"
                aria-label="Remove {item.track.title} from the queue"
                title="Remove from queue"
                onclick={(event) => {
                  releasePointerFocus(event);
                  void player.removeQueueEntry(item.key);
                }}
              >
                <XIcon size={16} aria-hidden="true" />
              </button>
              {/if}
            </li>
          {/if}
        {/each}
        {#if drag}
          <li class="queue-drop" aria-hidden="true" style:top="{drag.gap * ROW_HEIGHT}px"></li>
        {/if}
      </ol>
    </div>
  {/if}

  <p class="sr-only" role="status">{announcement}</p>
</aside>
