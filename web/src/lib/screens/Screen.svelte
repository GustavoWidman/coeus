<script lang="ts">
	import type { Route } from '#lib/router.svelte.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import LiveTag from '#lib/ui/LiveTag.svelte';
	import Nav from '#lib/ui/Nav.svelte';
	import Center from './Center.svelte';

	let { route, onback }: { route: Route; onback: () => void } = $props();

	let scrolled = $state(false);

	const title = $derived(
		route.screen === 'machine' || route.screen === 'log'
			? route.host
			: route.screen === 'deployment'
				? 'Deployment'
				: 'Fleet'
	);
</script>

<div class="screen" class:scrolled>
	<div class="body">
		<Nav {title} back={route.screen === 'fleet' ? undefined : 'Fleet'} {onback}>
			{#snippet right()}{#if fleet.ready}<LiveTag />{/if}{/snippet}
		</Nav>
		<div class="scroll" onscroll={(e) => (scrolled = e.currentTarget.scrollTop > 40)}>
			<Center {route} />
		</div>
	</div>
	<div class="shade"></div>
</div>
