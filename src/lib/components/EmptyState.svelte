<script lang="ts">
  import type { Component, Snippet } from 'svelte';
  import type { IconComponentProps } from 'phosphor-svelte';

  interface Props {
    Icon: Component<IconComponentProps>;
    title: string;
    children: Snippet;
  }

  let { Icon, title, children }: Props = $props();
</script>

<div class="mt-16 flex max-w-xl items-start gap-6">
  <div class="lamp" aria-hidden="true">
    <Icon size={30} weight="duotone" />
  </div>
  <div class="min-w-0 pt-2">
    <h2 class="font-display text-ink text-2xl font-semibold tracking-tight text-balance break-words">
      {title}
    </h2>
    <p class="text-ink-soft mt-2 max-w-[46ch] text-[15px] leading-relaxed text-pretty">
      {@render children()}
    </p>
  </div>
</div>
