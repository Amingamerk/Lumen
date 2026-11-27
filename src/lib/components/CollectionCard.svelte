<script lang="ts">
  import type { CollectionSummary } from '../ipc';
  import { SOURCE_NAMES } from '../sources';
  import CoverArt from './CoverArt.svelte';
  import SourceBadge from './SourceBadge.svelte';

  interface Props {
    collection: CollectionSummary;
    onopen: (id: string) => void;
  }

  let { collection, onopen }: Props = $props();

  const uid = $props.id();

  /** Rust owns the route, so the link never changes the page by itself. */
  function open(event: MouseEvent) {
    event.preventDefault();
    onopen(collection.id);
  }
</script>

<a
  class="card"
  href="#playlist/{collection.id}"
  draggable="false"
  title={collection.title}
  aria-label="{collection.title}, {collection.kind} from {SOURCE_NAMES[collection.source]}"
  aria-describedby="{uid}-subtitle"
  onclick={open}
>
  <CoverArt title={collection.title} />
  <span class="card-title">{collection.title}</span>
  <span class="card-meta">
    <SourceBadge source={collection.source} badge={collection.badge} />
    <span id="{uid}-subtitle" class="min-w-0 truncate">{collection.subtitle}</span>
  </span>
</a>
