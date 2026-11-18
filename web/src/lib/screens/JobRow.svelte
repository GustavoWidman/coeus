<script lang="ts">
	import { href } from '#lib/router.svelte.ts';
	import { ago, dur, origin, source } from '#lib/derive/format.ts';
	import { OUTCOME_DOT, outcome } from '#lib/derive/status.ts';
	import { ghost, enter } from '#lib/motion/flip.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Dot from '#lib/ui/Dot.svelte';
	import type { Hostname, JobCard } from '#lib/wire/types.ts';

	/** `moves` false: the row comes and goes with its list instead of on its own */
	let { host, job, moves = true }: { host: Hostname; job: JobCard; moves?: boolean } = $props();

	const done = $derived(job.state.kind === 'finished' ? job.state.outcome : null);
	const dot = $derived(done ? OUTCOME_DOT[done.kind] : 'wait');
	const took = $derived(
		job.started_at !== null && job.finished_at !== null ? job.finished_at - job.started_at : 0
	);
	const fresh = $derived(
		fleet.landed.some((l) => l.host === host && l.job === job.id && now.ms - l.at < 1400)
	);
</script>

<li data-key="j-{job.id}" in:enter={moves} out:ghost={moves}>
	<a class="jrow tap" class:fresh href={href({ screen: 'log', host, job: job.id })}>
		<span class="jid">#{job.id}</span>
		<div style="min-width:0">
			<div class="jh">
				<span class="act">{job.action}</span><span class="src">{source(job.source)}</span>
			</div>
			<div class="js" class:bad={dot === 'bad'}>
				<Dot s={dot} /><span>{done ? outcome(done) : `queued ${ago(job.ordered_at, now.ms)}`}</span>
			</div>
		</div>
		<span class="jorg">{origin(job.origin)}</span>
		<div class="jr">
			{done ? ago(job.finished_at ?? job.ordered_at, now.ms) : 'next'}<br /><span>
				{done && took ? dur(took) : '—'}
			</span>
		</div>
	</a>
</li>
