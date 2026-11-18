<!--
	One job's log: what the job is and how it stands, then its lines, followed live. Lines live
	outside Svelte's reactivity: `Tail` draws the few on screen straight into the list.
-->
<script lang="ts">
	import { device } from '#lib/auth/device.ts';
	import { clk, dur, source } from '#lib/derive/format.ts';
	import { OUTCOME_DOT, outcome } from '#lib/derive/status.ts';
	import { Lines } from '#lib/log/lines.ts';
	import { follow } from '#lib/log/stream.ts';
	import { Tail } from '#lib/log/tail.ts';
	import { reduced } from '#lib/motion/reduced.ts';
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
	/** every host is down, so the log can't be read and the job stands as last seen */
	const down = $derived(fleet.down !== null);
	/** nothing has answered for the log yet, not even with why it can't be read */
	const waiting = $derived(!started && failed === null && !down);
	const queued = $derived(!done && job?.state.kind === 'queued');
	const said = $derived.by(() => {
		if (done) {
			const took = job?.started_at && job.finished_at ? job.finished_at - job.started_at : 0;
			return outcome(done).split(':')[0] + (took ? ` · ${dur(took)}` : '');
		}
		return job?.state.kind === 'running' ? job.state.phase : 'queued';
	});

	// hidden, the pill would still cost the compositor a layer, so it leaves the page once it
	// has faded out, and comes back a frame before it fades in
	let pill = $state<'gone' | 'off' | 'on'>('gone');
	$effect(() => {
		if (!unseen) {
			if (pill === 'on') pill = reduced() ? 'gone' : 'off';
			return;
		}
		if (pill === 'gone') {
			pill = 'off';
			requestAnimationFrame(() => requestAnimationFrame(() => unseen && (pill = 'on')));
		} else pill = 'on';
	});

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
			failed: (why) => (failed = why),
			sign: (path) => device.headers('GET', path)
		});
		return () => {
			stop();
			t.destroy();
		};
	});
</script>

<div class="logp" aria-busy={waiting}>
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
				<Dot s={down ? 'stale' : done ? OUTCOME_DOT[done.kind] : running ? 'run' : 'wait'} />
				{host} · {said}{#if running && job?.started_at != null}{' · '}<Tick
						still
						v={clk(now.ms - job.started_at)}
					/>{/if}
			</div>
		</div>
		<span class="r">
			{#if wide}
				<span class="live">
					{down ? 'cached' : running ? 'tailing' : queued ? 'queued' : 'complete'}
				</span>
			{:else}
				<button type="button" class="done-btn" onclick={onclose}>Done</button>
			{/if}
		</span>
	</div>
	<div class="logbody mono" bind:this={box}>
		{#if empty}
			<p class="empty">
				{down
					? `No host answered (${fleet.down}). The log comes back with them.`
					: failed
						? `Could not read the log: ${failed}. Trying again.`
						: queued
							? 'Nothing yet: the job starts after the one before it.'
							: 'Waiting for the first line.'}
			</p>
		{/if}
		<div class="lns" bind:this={list}></div>
	</div>
	<button
		type="button"
		class="jump"
		class:on={pill === 'on'}
		hidden={pill === 'gone'}
		onclick={() => tail?.jump()}
		ontransitionend={() => unseen || (pill = 'gone')}
	>
		↓ <span class="jn">{unseen}</span> new lines
	</button>
</div>
