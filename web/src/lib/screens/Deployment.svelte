<script lang="ts">
	import { href } from '#lib/router.svelte.ts';
	import { clk, delta, dur, hms, origin, source } from '#lib/derive/format.ts';
	import {
		columns,
		counts,
		fraction,
		outcome,
		share,
		status,
		verdictDot
	} from '#lib/derive/status.ts';
	import { ghost, reveal, withFlip } from '#lib/motion/flip.ts';
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

	type Rows = typeof import('./DiffRows.svelte');
	/** the open rows' hosts, and what reading each one's diff came to */
	let open = $state.raw<string[]>([]);
	let read = $state.raw<Record<string, import('./DiffRows.svelte').Read>>({});
	let rows = $state.raw<Rows | null>(null);

	/** a row opens once its job has built, when the diff it wrote can be read */
	const opens = (r: DeploymentRow) =>
		r.job !== null &&
		(r.diff !== null ||
			r.verdict === 'done' ||
			r.verdict === 'caught_up' ||
			r.verdict === 'failed');

	async function toggle(r: DeploymentRow) {
		const key = `${r.host}/${r.job}`;
		if (open.includes(r.host)) return withFlip(() => (open = open.filter((h) => h !== r.host)));
		withFlip(() => (open = [...open, r.host]));
		const mod = rows ?? (await import('./DiffRows.svelte'));
		const got = read[key] && 'diff' in read[key] ? read[key] : await mod.load(r.host, r.job!);
		withFlip(() => {
			rows = mod;
			read = { ...read, [key]: got };
		});
	}

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
							still
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
							still
							v={clk(now.ms - w.started)}
						/>{:else}waiting{/if}
				</Label>
				<ul>
					{#each w.rows as r (r.host)}
						{@const v = verdict(r)}
						{@const can = opens(r)}
						{@const on = can && open.includes(r.host)}
						{@const got = read[`${r.host}/${r.job}`]}
						<li data-key="v-{r.host}" class:open={on}>
							{#if can}
								<button type="button" class="vrow tap" aria-expanded={on} onclick={() => toggle(r)}>
									{@render row(r, v, can)}
								</button>
							{:else}
								<a class="vrow tap" href={href({ screen: 'machine', host: r.host })}>
									{@render row(r, v, can)}
								</a>
							{/if}
							{#if on}
								<div class="diff" data-key="df-{r.host}" in:reveal out:ghost>
									{#if got && 'diff' in got && rows}
										<rows.default diff={got.diff} />
									{:else}
										<div class="dsum">
											<span>
												{got && 'error' in got
													? `Could not read the diff: ${got.error}.`
													: 'Reading the closure diff…'}
											</span>
										</div>
									{/if}
								</div>
							{/if}
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

{#snippet row(r: DeploymentRow, v: string, can: boolean)}
	<Dot s={verdictDot(r.verdict, fleet.machine(r.host))} />
	<div style="min-width:0">
		<span class="host">{r.host}</span><span class="vt" class:bad={r.verdict === 'failed'}>
			{v}
		</span>
	</div>
	<span class="dz">{r.diff ? `${delta(r.diff.size_delta)} MiB` : ''}</span><span class="pm">
		{can ? '+' : ''}
	</span>
	<Line
		rule
		p={share(r, fleet.machine(r.host), fleet.progress[r.host])}
		c={r.verdict === 'failed'
			? 'bad'
			: r.verdict === 'done' || r.verdict === 'caught_up'
				? 'done'
				: ''}
	/>
{/snippet}
