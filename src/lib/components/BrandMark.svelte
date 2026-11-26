<!-- Lumen's mark: three arcs, one per source, around a single light. -->
<script lang="ts">
  let { size = 32 }: { size?: number } = $props();
</script>

<svg class="brand-mark" width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
  <g fill="none" stroke-width="4.2" stroke-linecap="round">
    <path d="M8.64 7.83A11 11 0 0 1 23.36 7.83" stroke="var(--color-lume-hi)" />
    <path d="M26.76 13.71A11 11 0 0 1 19.4 26.46" stroke="var(--color-lume)" />
    <path d="M12.6 26.46A11 11 0 0 1 5.24 13.71" stroke="var(--color-ember)" />
  </g>
  <circle cx="16" cy="16" r="4.4" fill="var(--color-ink)" />
</svg>
