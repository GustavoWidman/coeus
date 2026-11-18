<script lang="ts">
	import Console from '#lib/screens/Console.svelte';
	import Stack from '#lib/screens/Stack.svelte';

	const wide = matchMedia('(min-width: 1024px)');
	let isWide = $state(wide.matches);
	wide.addEventListener('change', (e) => (isWide = e.matches));
</script>

{#if isWide}<Console />{:else}<Stack />{/if}
