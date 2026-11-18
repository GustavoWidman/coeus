<script lang="ts">
	import { href } from '#lib/router.svelte.ts';
	import { clk, dur, hms, origin, source } from '#lib/derive/format.ts';
	import {
		columns,
		counts,
		fraction,
		outcome,
		share,
		status,
		verdictDot
	} from '#lib/derive/status.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Dot from '#lib/ui/Dot.svelte';
	import Label from '#lib/ui/Label.svelte';
	import Line from '#lib/ui/Line.svelte';
	import Tick from '#lib/ui/Tick.svelte';
	import type { DeploymentRow, DeploymentView } from '#lib/wire/types.ts';

	let { d }: { d: DeploymentView } = $props();

	const c = $derived(counts(d));
	const cols = $derived(columns(d));

	function verdict(r: DeploymentRow) {
		const card = fleet.machine(r.host);
		const st = card?.state;
		const job = st?.running;
		const p = fleet.progress[r.host];
		const pct = () =>
			job?.state.kind === 'running'
				? `${job.state.phase} · ${Math.round(fraction(job, p) * 100)}%`
				: 'starting';
		switch (r.verdict) {
			case 'done':
				return `done · generation ${st?.generation ?? '—'}`;
			case 'caught_up':
				return `caught up · generation ${st?.generation ?? '—'}`;
			case 'running':
				return pct();
			case 'catching_up':
				return `catching up · ${pct()}`;
			case 'waiting':
				return 'waiting for this wave';
			case 'failed': {
				const ended = st?.recent.find((j) => j.id === r.job)?.state;
				return ended?.kind === 'finished' ? outcome(ended.outcome) : 'failed';
			}
			case 'pending':
				return card?.link.kind === 'unreachable'
					? status(card, p, now.ms).text
					: `pending · follows ${st?.follow?.git_ref ?? 'its ref'}`;
		}
	}
</script>

<div class="ctr">
	<div class="lt">
		<div class="kicker">deployment · {origin(d.origin)}</div>
		<h1 class="mt">{source(d.source)}</h1>
		{#if d.subject}<p class="sub">{d.subject}</p>{/if}
		<div class="meta">
			<span>started <b>{hms(d.ordered_at)}</b></span><span>
				{d.settled_at ? 'took' : 'elapsed'}
				<b>
					{#if d.settled_at}{dur(d.settled_at - d.ordered_at)}{:else}<Tick
							v={clk(now.ms - d.ordered_at)}
						/>{/if}
				</b>
			</span><span>{d.waves.length} {d.waves.length === 1 ? 'wave' : 'waves'}</span>
		</div>
	</div>
	<div class="stats">
		{@render stat('done', c.done, '')}{@render stat(
			'running',
			c.run,
			c.run ? 'run' : ''
		)}{@render stat('failed', c.fail, c.fail ? 'bad' : '')}{@render stat('pending', c.pend, '')}
	</div>
	<div class="waves-d">
		{#each cols as w (w.name)}
			<section class="sec" data-key="w-{w.name}">
				<Label a={w.name}>
					{#if !w.ordered}{w.done
							? 'caught up'
							: 'follows the ref'}{:else if w.settled !== null && w.started !== null}done in {dur(
							w.settled - w.started
						)}{:else if w.started !== null}running · <Tick
							v={clk(now.ms - w.started)}
						/>{:else}waiting{/if}
				</Label>
				<ul>
					{#each w.rows as r (r.host)}
						{@const v = verdict(r)}
						<li data-key="v-{r.host}">
							<a class="vrow tap" href={href({ screen: 'machine', host: r.host })}>
								<Dot s={verdictDot(r.verdict, fleet.machine(r.host))} />
								<div style="min-width:0">
									<span class="host">{r.host}</span><span
										class="vt"
										class:bad={r.verdict === 'failed'}
									>
										{v}
									</span>
								</div>
								<span class="dz"></span><span class="pm"></span>
								<Line
									rule
									p={share(r, fleet.machine(r.host), fleet.progress[r.host])}
									c={r.verdict === 'failed'
										? 'bad'
										: r.verdict === 'done' || r.verdict === 'caught_up'
											? 'done'
											: ''}
								/>
							</a>
						</li>
					{/each}
				</ul>
			</section>
		{/each}
	</div>
</div>

{#snippet stat(label: string, n: number, cl: string)}
	<div class="stat {cl}">
		<div class="mlab">{label}</div>
		<div class="mval"><Tick v={n} /></div>
	</div>
{/snippet}
