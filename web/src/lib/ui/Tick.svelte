<script lang="ts">
	import { roll } from '#lib/motion/pulse.ts';

	/** `still`: the number changes in place, as an elapsed time does every second */
	let { v, still = false }: { v: string | number; still?: boolean } = $props();

	let el: HTMLSpanElement;
	let shown: string | null = null;

	$effect(() => {
		const next = String(v);
		if (next === shown) return;
		if (still) el.textContent = next;
		else roll(el, shown, next);
		shown = next;
	});
</script>

<span class="tk" bind:this={el}></span>
