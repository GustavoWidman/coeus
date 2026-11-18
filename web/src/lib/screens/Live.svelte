<!-- The fleet's screens: a stack of them on a phone, a console on a wide screen. -->
<script lang="ts">
	import '#lib/stream/run.ts';
	import Console from './Console.svelte';
	import Stack from './Stack.svelte';

	const wide = matchMedia('(min-width: 1024px)');
	let isWide = $state(wide.matches);
	$effect(() => {
		const change = (e: MediaQueryListEvent) => (isWide = e.matches);
		wide.addEventListener('change', change);
		return () => wide.removeEventListener('change', change);
	});
</script>

{#if isWide}<Console />{:else}<Stack />{/if}
