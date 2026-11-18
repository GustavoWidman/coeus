<script lang="ts">
	import type { Route } from '#lib/router.svelte.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import Deployment from './Deployment.svelte';
	import Fleet from './Fleet.svelte';
	import Machine from './Machine.svelte';

	let { route }: { route: Route } = $props();

	const deployment = $derived(
		route.screen === 'deployment'
			? fleet.deployments.find((d) => d.order === route.order)
			: undefined
	);
</script>

{#if route.screen === 'machine' || route.screen === 'log'}
	<Machine host={route.host} />
{:else if route.screen === 'deployment'}
	{#if deployment}
		<Deployment d={deployment} />
	{:else if fleet.ready}
		<div class="ctr">
			<div class="lt">
				<div class="kicker">deployment</div>
				<h1 class="mt">{route.order}</h1>
				<p class="sub">No deployment by this name ran in the last day.</p>
			</div>
		</div>
	{/if}
{:else}
	<Fleet />
{/if}
