<script lang="ts">
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import Dot from './Dot.svelte';

	const live = $derived(fleet.link === 'live');
</script>

<span class="live">
	<Dot s={live ? 'run' : 'wait'} beat={fleet.beat} />{live ? 'live' : 'connecting'}
</span>
