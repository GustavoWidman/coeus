<script lang="ts">
	import { href } from '#lib/router.svelte.ts';
	import { num, plat } from '#lib/derive/format.ts';
	import { status } from '#lib/derive/status.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Dot from '#lib/ui/Dot.svelte';
	import Line from '#lib/ui/Line.svelte';
	import Tick from '#lib/ui/Tick.svelte';
	import type { MachineCard } from '#lib/wire/types.ts';

	let { card, sel }: { card: MachineCard; sel: boolean } = $props();

	const s = $derived(status(card, fleet.progress[card.host], now.ms));
	const st = $derived(card.state);
	/** every host is down, so the row is the machine as it last was */
	const off = $derived(fleet.link === 'down');
</script>

<li data-key="r-{card.host}">
	<a class="row tap" class:sel href={href({ screen: 'machine', host: card.host })}>
		<Dot s={off ? 'stale' : s.dot} />
		<div style="min-width:0">
			<div class="row-h">
				<span class="host">{card.host}</span><span class="plat">{st ? plat(st) : ''}</span>
			</div>
			<div class="row-s {off ? 'mute' : (s.tone ?? '')}">
				{s.text}{#if s.builds}{' · '}<span class="mono">
						<Tick v={num(s.builds.done)} /><span class="m">/{num(s.builds.expected)}</span>
					</span>{/if}
			</div>
		</div>
		<div class="row-r">
			{#if st?.generation != null}<Tick v={st.generation} />{:else}—{/if}<small>
				gen{#if st?.reboot_required}{' · '}<span class="flag">
						reboot
					</span>{:else if st?.pending_boot}{' · '}<span class="flag">boot</span>{/if}
			</small>
		</div>
		<Line rule p={s.p ?? 0} />
	</a>
</li>
