<!--
	One job's log: what the job is and how it stands, then its lines, followed live. Lines live
	outside Svelte's reactivity: `Tail` draws the few on screen straight into the list.
-->
<script lang="ts">
	import { clk, dur, source } from '#lib/derive/format.ts';
	import { OUTCOME_DOT, outcome } from '#lib/derive/status.ts';
	import { Lines } from '#lib/log/lines.ts';
	import { follow } from '#lib/log/stream.ts';
	import { Tail } from '#lib/log/tail.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Dot from '#lib/ui/Dot.svelte';
	import Tick from '#lib/ui/Tick.svelte';
	import type { JobCard, OutcomeView } from '#lib/wire/types.ts';

	let {
		host,
		id,
		wide = false,
		onclose
	}: { host: string; id: number; wide?: boolean; onclose?: () => void } = $props();

	let started = $state.raw<JobCard | null>(null);
	let ended = $state.raw<OutcomeView | null>(null);
	let empty = $state(true);
	let failed = $state<string | null>(null);
	let unseen = $state(0);

	/** the job as the fleet last described it, which moves on as the job does */
	const job = $derived.by(() => {
		const st = fleet.machine(host)?.state;
		const known = st && [st.running, st.next, ...st.recent].find((j) => j?.id === id);
		return known ?? started;
	});
	const done = $derived(ended ?? (job?.state.kind === 'finished' ? job.state.outcome : null));
	const running = $derived(!done && job?.state.kind === 'running');
	const queued = $derived(!done && job?.state.kind === 'queued');

	let box: HTMLElement;
	let list: HTMLElement;
	let tail: Tail | undefined;

	$effect(() => {
		const lines = new Lines();
		const t = (tail = new Tail(box, list, lines, (n) => (unseen = n)));
		const stop = follow(`/api/machines/${encodeURIComponent(host)}/jobs/${id}/log`, {
			started: (j) => (started = j),
			lines(add) {
				t.grew(add.length, lines.add(add));
				if (add.length) empty = false;
			},
			ended: (o) => (ended = o),
			failed: (why) => (failed = why)
		});
		return () => {
			stop();
			t.destroy();
		};
	});
</script>

<div class="logp">
	<div class="logh">
		<div style="min-width:0">
			<div class="t">
				#{id}
				{job?.action ?? ''}
				{#if job}<span class="mono" style="font-weight:400;font-size:13px;color:var(--ink2)">
						{source(job.source)}
					</span>{/if}
			</div>
			<div class="s">
				<Dot s={done ? OUTCOME_DOT[done.kind] : running ? 'run' : 'wait'} />
				{host} · {#if done}{outcome(done).split(':')[0]}{job?.started_at && job.finished_at
						? ` · ${dur(job.finished_at - job.started_at)}`
						: ''}{:else if running && job?.state.kind === 'running'}{job.state
						.phase}{#if job.started_at !== null}
						· <Tick still v={clk(now.ms - job.started_at)} />{/if}{:else}queued{/if}
			</div>
		</div>
		<span class="r">
			{#if wide}
				<span class="live">{running ? 'tailing' : queued ? 'queued' : 'complete'}</span>
			{:else}
				<button type="button" class="done-btn" onclick={onclose}>Done</button>
			{/if}
		</span>
	</div>
	<div class="logbody mono" bind:this={box}>
		{#if empty}
			<p class="empty">
				{failed
					? `Could not read the log: ${failed}. Trying again.`
					: queued
						? 'Nothing yet: the job starts after the one before it.'
						: 'Waiting for the first line.'}
			</p>
		{/if}
		<div class="lns" bind:this={list}></div>
	</div>
	<button type="button" class="jump" class:on={unseen > 0} onclick={() => tail?.jump()}>
		↓ <span class="jn">{unseen}</span> new lines
	</button>
</div>
