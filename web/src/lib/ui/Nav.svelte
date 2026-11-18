<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		back,
		onback,
		right
	}: { title: string; back?: string; onback?: () => void; right?: Snippet } = $props();
</script>

<header class="nav">
	<div>
		{#if back}
			<a class="back tap" href="/" onclick={(e) => (e.preventDefault(), onback?.())}>
				<svg viewBox="0 0 12 20" width="10" height="17">
					<path
						d="M10 2 2 10l8 8"
						fill="none"
						stroke="currentColor"
						stroke-width="2.2"
						stroke-linecap="round"
						stroke-linejoin="round"
					/>
				</svg><span>{back}</span>
			</a>
		{:else}
			<span class="wm">coeus</span>
		{/if}
	</div>
	<div class="nav-t">{title}</div>
	<div class="nav-r">{@render right?.()}</div>
</header>
