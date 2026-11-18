<script lang="ts">
	import { href } from '#lib/router.svelte.ts';
	import { clk, hms, mib, num, origin, source } from '#lib/derive/format.ts';
	import { PHASES, progressOf } from '#lib/derive/status.ts';
	import { enter, ghost } from '#lib/motion/flip.ts';
	import { pulse } from '#lib/motion/pulse.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Label from '#lib/ui/Label.svelte';
	import Line from '#lib/ui/Line.svelte';
	import Tick from '#lib/ui/Tick.svelte';
	import type { Hostname, JobCard, TallyView } from '#lib/wire/types.ts';

	let { host, job }: { host: Hostname; job: JobCard } = $props();

	const running = $derived(job.state.kind === 'running');
	const at = $derived(job.state.kind === 'running' ? PHASES.indexOf(job.state.phase) : -1);
	const p = $derived(at >= 3 ? progressOf(job, fleet.progress[host]) : undefined);
	/** nix can build two derivations of one name at once, so each gets its count as a key */
	const active = $derived(
		(p?.active ?? []).map((name, i, all) => ({
			name,
			key: `${name}#${all.slice(0, i).filter((n) => n === name).length}`
		}))
	);
</script>

{#snippet meter(label: string, t: TallyView | undefined)}
	<div class="meter">
		<div class="mlab">{label}</div>
		<div class="mval">
			{#if t}<Tick v={num(t.done)} />{:else}<span class="m">—</span>{/if}<span class="of">
				/{t ? num(t.expected) : '—'}
			</span>
		</div>
		<Line
			p={t && t.expected ? t.done / t.expected : 0}
			c={t && t.done >= t.expected ? 'done' : ''}
		/>
		<div class="mfoot">
			{#if !p}{label === 'builds'
					? 'starts after evaluation'
					: '—'}{:else if label === 'builds'}<span class={p.builds.running ? 'hot' : ''}>
					<Tick v={p.builds.running} /> running
				</span>
				· {p.builds.failed} failed{:else}<Tick v={mib(p.downloaded_bytes)} /> / {mib(
					p.expected_bytes
				)} MiB{/if}
		</div>
	</div>
{/snippet}

<section class="sec" data-key="run-{job.id}" in:enter out:ghost>
	<Label a={running ? 'Running' : 'Starting'}>
		#{job.id} · {#if running && job.started_at !== null}<Tick
				still
				v={clk(now.ms - job.started_at)}
			/>{:else}queued{/if}
	</Label>
	<div class="jhead">
		<span class="act">{job.action}</span><span class="src">{source(job.source)}</span><span
			class="org"
		>
			{origin(job.origin)}{running && job.started_at !== null
				? ` · started ${hms(job.started_at)}`
				: ''}
		</span>
	</div>
	<div class="runwrap">
		<ol class="phases" style:--f={running ? (at / 5).toFixed(3) : 0}>
			<span class="fill"></span>
			{#each PHASES as name, i (name)}
				{@const cl = i < at ? 'done' : i === at ? 'cur' : ''}
				<li class="ph {cl}" data-s={cl || 'todo'} {@attach pulse(cl)}>
					<i><b></b></i><span>{name}</span><span class="mono">
						{#if cl === 'cur' && job.phase_started_at !== null}<Tick
								still
								v={clk(now.ms - job.phase_started_at)}
							/>{:else if cl === ''}—{/if}
					</span>
				</li>
			{/each}
		</ol>
		<div class="meters">
			{@render meter('builds', p?.builds)}{@render meter('downloads', p?.downloads)}
		</div>
		<div>
			<div class="mlab">building now</div>
			<ul class="drvs">
				{#each active as { name, key } (key)}
					<li data-key="d-{key}" in:enter out:ghost><i class="busy"></i><span>{name}</span></li>
				{:else}
					<li class="none">
						{!running
							? 'Starting in a moment.'
							: at < 3
								? 'Derivations show here once nix starts building.'
								: 'Nothing building. Activation in progress.'}
					</li>
				{/each}
			</ul>
		</div>
	</div>
	<a class="loglink tap" href={href({ screen: 'log', host, job: job.id })}>
		<span>Live log</span><span class="mono">#{job.id}</span><span class="chev">›</span>
	</a>
</section>
