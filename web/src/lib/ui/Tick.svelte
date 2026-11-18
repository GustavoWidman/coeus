<script lang="ts">
	import { roll } from '#lib/motion/pulse.ts';

	let { v }: { v: string | number } = $props();

	let el: HTMLSpanElement;
	let shown: string | null = null;

	$effect(() => {
		const next = String(v);
		if (next === shown) return;
		roll(el, shown, next);
		shown = next;
	});
</script>

<span class="tk" bind:this={el}></span>
