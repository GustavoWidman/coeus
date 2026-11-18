<script lang="ts">
	import { href } from '#lib/router.svelte.ts';
	import { clk, dur, origin, source } from '#lib/derive/format.ts';
	import { columns, counts, share, verdictDot } from '#lib/derive/status.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Dot from '#lib/ui/Dot.svelte';
	import Label from '#lib/ui/Label.svelte';
	import Line from '#lib/ui/Line.svelte';
	import Tick from '#lib/ui/Tick.svelte';
	import type { DeploymentView } from '#lib/wire/types.ts';

	let { d }: { d: DeploymentView } = $props();

	const c = $derived(counts(d));
	const cols = $derived(columns(d));
	const current = $derived(cols.findIndex((w) => w.ordered && !w.done));
	const ordered = $derived(cols.filter((w) => w.ordered).length);
</script>

<a class="dep tap" href={href({ screen: 'deployment', order: d.order })}>
	<Label a="Deployment">
		{#if d.settled_at}done{:else}<Tick still v={clk(now.ms - d.ordered_at)} />{/if}
	</Label>
	<div class="dep-h">
		<span class="src">{source(d.source)}</span><span class="st">
			{d.settled_at
				? `complete · ${dur(d.settled_at - d.ordered_at)}`
				: `${cols[current]?.name} · ${current + 1} of ${ordered}`}
		</span>
	</div>
	<div class="dep-sub">
		{d.subject ?? origin(d.origin)} · {c.done} done{c.fail ? ` · ${c.fail} failed` : ''}
	</div>
	<div class="waves">
		{#each cols as w, i (w.name)}
			<div
				class="wv {w.done ? 'done' : i === current && !d.settled_at ? 'cur' : ''}"
				style:flex={w.rows.length + 1}
			>
				<Line
					p={w.rows.reduce(
						(a, r) => a + share(r, fleet.machine(r.host), fleet.progress[r.host]),
						0
					) / w.rows.length}
					c={w.done ? 'done' : ''}
				/><span class="wn">{w.name}</span>
				{#each w.rows as r (r.host)}
					<span class="vc">
						<Dot s={verdictDot(r.verdict, fleet.machine(r.host))} /><span>{r.host}</span>
					</span>
				{/each}
			</div>
		{/each}
	</div>
</a>
