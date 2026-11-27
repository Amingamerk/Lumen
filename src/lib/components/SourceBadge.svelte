<script lang="ts">
  import type { SourceTag } from '../ipc';
  import { SOURCE_NAMES } from '../sources';

  let { source, badge }: SourceTag = $props();
</script>

<span class="badge" data-source={source} title={SOURCE_NAMES[source]} translate="no">
  {badge}
</span>
