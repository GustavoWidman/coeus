<script lang="ts">
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import Dot from './Dot.svelte';

	const tag = $derived(
		fleet.link === 'live'
			? ({ s: 'run', t: 'live' } as const)
			: fleet.link === 'down'
				? ({ s: 'stale', t: 'cached' } as const)
				: ({ s: 'wait', t: 'connecting' } as const)
	);
</script>

<span class="live"><Dot s={tag.s} beat={fleet.beat} />{tag.t}</span>
