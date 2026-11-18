<script lang="ts">
	import { ago, hms, num, short, source } from '#lib/derive/format.ts';
	import { status } from '#lib/derive/status.ts';
	import { enter, ghost } from '#lib/motion/flip.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Dot from '#lib/ui/Dot.svelte';
	import Label from '#lib/ui/Label.svelte';
	import Tick from '#lib/ui/Tick.svelte';
	import JobRow from './JobRow.svelte';
	import RunSection from './RunSection.svelte';

	let { host }: { host: string } = $props();

	const card = $derived(fleet.machine(host));
	const st = $derived(card?.state ?? null);
	const s = $derived(card && status(card, fleet.progress[card.host], now.ms));
	/** the job the run section shows: the running one, else the one about to start */
	const job = $derived(st?.running ?? st?.next ?? null);
	const queued = $derived(st?.running ? st.next : null);
	const store = $derived.by(() => {
		const name = st?.current?.slice('/nix/store/'.length) ?? '';
		const dash = name.indexOf('-');
		return { hash: name.slice(0, 8), name: name.slice(dash + 1) };
	});
</script>

{#if card && s}
	<div class="ctr">
		<div class="lt">
			<div class="kicker">{st ? `${st.platform} · ${st.system}` : 'machine'}</div>
			<h1>{host}</h1>
			<p class="status">
				<Dot s={s.dot} /><span>
					{s.text}{#if s.builds}{' · '}<span class="mono">
							<Tick v={num(s.builds.done)} /><span class="m">/{num(s.builds.expected)}</span>
						</span>{/if}
				</span>
			</p>
		</div>
		<div class="mbody">
			{#if job}
				{#key job.id}<RunSection host={card.host} {job} />{/key}
			{:else}
				<section class="sec" data-key="idle" in:enter out:ghost>
					<Label a="Running" />
					<p class="empty">
						{#if card.link.kind === 'unreachable'}
							{card.link.reason.kind === 'offline'
								? `Asleep since ${hms(st?.seen_at ?? card.link.since)}. coeusd catches up to ${st?.follow?.git_ref ?? 'deployed'} when the machine wakes.`
								: `Out of reach since ${hms(card.link.since)}: ${card.link.reason.detail}.`}
						{:else if !st}
							Not reached yet.
						{:else if st.follow}
							Idle. coeusd follows {st.follow.git_ref} and starts a job when it moves.
						{:else}
							Idle.
						{/if}
					</p>
				</section>
			{/if}
			<section class="sec half" data-key="q">
				<Label a="Queue">{queued ? 1 : 0}</Label>
				{#if queued}
					<ul class="jobs"><JobRow host={card.host} job={queued} moves={false} /></ul>
				{:else}
					<p class="empty">Nothing queued.</p>
				{/if}
			</section>
			{#if st}
				<section class="sec half" data-key="gen">
					<Label a="Generation" />
					<dl class="kv">
						<div>
							<dt>generation</dt>
							<dd class="big">
								{#if st.generation !== null}<Tick v={st.generation} />{:else}—{/if}
							</dd>
						</div>
						<div>
							<dt>store path</dt>
							<dd>
								{#if st.current}<span class="m">/nix/store/</span>{store.hash}<span class="m">
										…-{store.name}
									</span>{:else}—{/if}
							</dd>
						</div>
						<div>
							<dt>source</dt>
							<dd>{st.deployed ? source(st.deployed) : '—'}</dd>
						</div>
						<div>
							<dt>follows</dt>
							<dd>
								{#if !st.follow}—{:else if st.follow.paused}<span class="w">
										paused
									</span>{st.deployed ? ` · ${source(st.deployed)} holds it` : ''}{:else}{st.follow
										.git_ref}{#if st.follow.latest}{` → ${short(st.follow.latest.rev)} `}<span
											class="m"
										>
											· committed {ago(st.follow.latest.committed_at, now.ms)}
										</span>{/if}{/if}
							</dd>
						</div>
						<div>
							<dt>boot</dt>
							<dd>
								{#if st.reboot_required}<span class="w">
										reboot required
									</span>{:else if st.pending_boot}<span class="w">pending boot</span>
									<span class="m">
										· generation {st.generation} on next boot
									</span>{:else}current{/if}
							</dd>
						</div>
						<div>
							<dt>coeusd</dt>
							<dd>
								{st.version}
								<span class="m">
									· {card.link.kind === 'connected'
										? 'reachable'
										: `seen ${ago(st.seen_at, now.ms)}`}
								</span>
							</dd>
						</div>
					</dl>
				</section>
				<section class="sec" data-key="hist">
					<Label a="History">last {st.recent.length}</Label>
					<ul class="jobs">
						{#each st.recent as j (j.id)}
							<JobRow host={card.host} job={j} />
						{/each}
					</ul>
				</section>
			{/if}
		</div>
	</div>
{:else if fleet.ready}
	<div class="ctr">
		<div class="lt">
			<div class="kicker">machine</div>
			<h1>{host}</h1>
			<p class="sub">No machine by this name is in the fleet.</p>
		</div>
	</div>
{/if}
