<!--
	The desktop: the fleet always on the left, and in the middle the machine picked from it, or
	else a deployment, the newest one across machines unless the address names another. On the
	right, a job's log: the one the address names, or else the job of the machine in
	the middle, or of the first machine building, or the one that last finished.
-->
<script lang="ts">
	import { untrack } from 'svelte';
	import { router, type Route } from '#lib/router.svelte.ts';
	import { hms } from '#lib/derive/format.ts';
	import { latest } from '#lib/derive/status.ts';
	import { enter, stage } from '#lib/motion/flip.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import LiveTag from '#lib/ui/LiveTag.svelte';
	import { boot } from './boot.svelte.ts';
	import Center from './Center.svelte';
	import Fleet from './Fleet.svelte';
	import type LogView from './Log.svelte';

	const route = $derived(router.nav.route);
	const host = $derived(route.screen === 'machine' || route.screen === 'log' ? route.host : null);
	const center = $derived.by((): Route | null => {
		if (host) return { screen: 'machine', host };
		if (route.screen === 'deployment' || route.screen === 'device') return route;
		const d = latest(fleet.deployments);
		return d ? { screen: 'deployment', order: d.order } : null;
	});
	const key = $derived(
		center?.screen === 'machine'
			? `m-${center.host}`
			: center?.screen === 'deployment'
				? `d-${center.order}`
				: center?.screen === 'device'
					? 'device'
					: '-'
	);

	const log = $derived.by((): { host: string; id: number } | null => {
		if (route.screen === 'log') return { host: route.host, id: route.job };
		const states = fleet.machines.flatMap((m) =>
			m.state && (!host || m.host === host) ? [{ host: m.host, st: m.state }] : []
		);
		const running = states.find((m) => m.st.running);
		if (running) return { host: running.host, id: running.st.running!.id };
		const last = states
			.flatMap(({ host, st }) => st.recent.slice(0, 1).map((j) => ({ host, j })))
			.sort((a, b) => (b.j.finished_at ?? 0) - (a.j.finished_at ?? 0))[0];
		if (last) return { host: last.host, id: last.j.id };
		const next = states.find((m) => m.st.next);
		return next ? { host: next.host, id: next.st.next!.id } : null;
	});

	let Log = $state.raw<typeof LogView | null>(null);
	$effect(() => {
		if (log && !Log) import('./Log.svelte').then((m) => (Log = m.default));
	});

	// the pane changes logs in a task of its own, not in the one where the fleet's update that
	// ended a job moves everything else
	const named = $derived(log ? `${log.host}/${log.id}` : '');
	let shown = $state.raw<{ host: string; id: number } | null>(null);
	$effect(() => {
		named;
		const next = untrack(() => log);
		const t = setTimeout(() => (shown = next));
		return () => clearTimeout(t);
	});

	/** the pane names a log it does not show yet */
	const loading = $derived(log !== null && (!Log || named !== `${shown?.host}/${shown?.id}`));

	let mid: HTMLElement;
	$effect(() => {
		key;
		untrack(() => (mid.scrollTop = 0));
	});

	const attach = (el: HTMLElement) => {
		stage.roots = [el];
		return () => (stage.roots = []);
	};
</script>

<div class="console" {@attach attach}>
	<header class="top">
		<span class="wm">coeus</span>
		<div class="crumbs">
			<span>fleet</span><span>/</span><b>
				{host ?? (route.screen === 'device' ? 'device' : 'deployment')}
			</b>
		</div>
		<span class="sp"></span>
		<LiveTag />
		<span class="clock">{hms(now.ms)}</span>
		{#if fleet.viewer}
			<a
				class="crumbs me"
				href="/device"
				aria-current={route.screen === 'device' ? 'page' : undefined}
				onpointerdown={() => import('./Device.svelte')}
			>
				{fleet.viewer.login.split('@')[0]} · {fleet.viewer.device}
			</a>
		{/if}
	</header>
	<aside class="pane left">
		<div class="scroll" class:offline={fleet.link === 'down'}>
			<Fleet wide selected={host} />
		</div>
	</aside>
	<main class="pane mid" bind:this={mid}>
		{#if boot.view === 'fleet'}
			{#key key}
				<div data-key="c-{key}" in:enter>
					<div class="scroll" class:offline={fleet.link === 'down'}>
						{#if center}
							<Center route={center} />
						{:else}
							<div class="ctr">
								<div class="lt">
									<div class="kicker">deployment</div>
									<p class="sub">No deployment reached more than one machine in the last day.</p>
								</div>
							</div>
						{/if}
					</div>
				</div>
			{/key}
		{/if}
	</main>
	<section class="pane right" aria-busy={loading}>
		{#if boot.view === 'fleet' && shown && Log}
			{#key `${shown.host}/${shown.id}`}
				<Log host={shown.host} id={shown.id} wide />
			{/key}
		{/if}
	</section>
</div>
