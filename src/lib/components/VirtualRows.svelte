<!--
  A list held in memory, drawn a window at a time: only the rows near the view are in the
  DOM. It scrolls with the page, in the nearest `[data-scroller]`. Every row is as tall as
  `rowHeight`; `row` draws one inside its `<li>`.
-->
<script lang="ts" generics="T">
  import type { Attachment } from 'svelte/attachments';
  import type { Snippet } from 'svelte';
  import { rowRange } from '../virtual';

  interface Props {
    items: T[];
    rowHeight: number;
    /** The id of the heading that names the list. */
    labelledby: string;
    /** A stable key for an item. */
    key: (item: T) => string;
    /** Extra classes for each row. */
    rowClass?: string;
    /** Extra attributes for each row. */
    rowData?: (item: T) => string | undefined;
    class?: string;
    row: Snippet<[T]>;
  }

  let { items, rowHeight, labelledby, key, rowClass = '', rowData, class: className = '', row }: Props =
    $props();

  const OVERSCAN = 8;

  let scrolled = $state.raw(0);
  let viewport = $state.raw(0);

  const range = $derived(
    rowRange({ scrolled, viewport, rowHeight, total: items.length, overscan: OVERSCAN }),
  );
  const shown = $derived(items.slice(range.start, range.end));

  /** Follows how much of the list is in view. */
  const follow: Attachment<HTMLElement> = (list) => {
    const scroller = list.closest<HTMLElement>('[data-scroller]');
    if (!scroller) return;

    const measure = () => {
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
</script>

<!-- As tall as the whole list, so the scrollbar is true. Rows are placed in it. -->
<ul
  class="vlist {className}"
  aria-labelledby={labelledby}
  style:height="{items.length * rowHeight}px"
  {@attach follow}
>
  {#each shown as item, at (key(item))}
    <li
      class="vrow {rowClass}"
      data-kind={rowData?.(item)}
      style:height="{rowHeight}px"
      style:transform="translateY({(range.start + at) * rowHeight}px)"
    >
      {@render row(item)}
    </li>
  {/each}
</ul>
