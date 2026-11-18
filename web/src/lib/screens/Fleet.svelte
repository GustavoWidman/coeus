<script lang="ts">
	import { latest } from '#lib/derive/status.ts';
	import { hms } from '#lib/derive/format.ts';
	import { E } from '#lib/motion/flip.ts';
	import { reduced } from '#lib/motion/reduced.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Label from '#lib/ui/Label.svelte';
	import { boot } from './boot.svelte.ts';
	import DeployStrip from './DeployStrip.svelte';
	import MachineRow from './MachineRow.svelte';

	/** in the desktop console's left pane, where the selected machine is marked */
	let { wide = false, selected = null }: { wide?: boolean; selected?: string | null } = $props();

	const SKELETON = [
		[46, 150],
		[96, 180],
		[34, 130],
		[80, 170],
		[62, 120],
		[62, 140]
	];

	const deployment = $derived(latest(fleet.deployments));
	const running = $derived(
		fleet.machines.filter((m) => m.state?.running?.state.kind === 'running').length
	);
	/** machines out of reach: asleep, which catch up when they wake, and the rest */
	const away = $derived.by(() => {
		let asleep = 0;
		let lost = 0;
		for (const m of fleet.machines)
			if (m.link.kind === 'unreachable') {
				if (m.link.reason.kind === 'offline') asleep++;
				else lost++;
			}
		return { asleep, lost };
	});

	const fade = (el: HTMLElement) => {
		if (boot.faded && !reduced())
			el.animate([{ opacity: 0.3 }, { opacity: 1 }], { duration: 200, easing: E });
	};
</script>

{#if boot.view === 'fleet'}
	<div class="lt">
		<h1>Fleet</h1>
		<p class="sub mono">
			{fleet.machines.length} machines · {running} running{away.asleep
				? ` · ${away.asleep} asleep`
				: ''}{away.lost ? ` · ${away.lost} unreachable` : ''}
		</p>
	</div>
	{#if deployment}<DeployStrip d={deployment} />{/if}
	<Label a="Machines">{fleet.machines.length}</Label>
	<ul class="rows" {@attach fade}>
		{#each fleet.machines as m (m.host)}
			<MachineRow card={m} sel={selected === m.host} />
		{/each}
	</ul>
	{#if !wide}
		<div class="foot">
			<span>served by {fleet.servedBy}</span><span>stream · {hms(now.ms)}</span>
		</div>
	{/if}
{:else if boot.view === 'skeleton'}
	<div class="lt">
		<h1>Fleet</h1>
		<p class="sub mono">connecting…</p>
	</div>
	<Label a="Machines" />
	<ul class="rows">
		{#each SKELETON as [a, b], i (i)}
			<li data-key="r-{i}">
				<div class="row">
					<i class="dot" data-s="idle"></i>
					<div>
						<span class="sk" style="width:{a}px;height:12px"></span><span
							class="sk"
							style="width:{b}px;height:9px;margin-top:9px"
						></span>
					</div>
					<span class="sk" style="width:34px;height:12px"></span>
					<div class="rule"></div>
				</div>
			</li>
		{/each}
	</ul>
{/if}
