<script lang="ts">
	import type { Snippet } from 'svelte';

	let { a, children }: { a: string; children?: Snippet } = $props();
</script>

<div class="lbl">
	<span>{a}</span>
	<hr />
	{#if children}<span class="v">{@render children()}</span>{/if}
</div>
