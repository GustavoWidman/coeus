<!--
	Signing in: the app asks the serving hosts' tailnet addresses every few seconds and signs
	itself in through the first that answers, then says until when the session lasts.
-->
<script lang="ts">
	import { onDestroy } from 'svelte';
	import { day } from '#lib/derive/format.ts';
	import { router } from '#lib/router.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Dot from '#lib/ui/Dot.svelte';
	import { noun, standalone as isStandalone } from '#lib/ui/platform.ts';
	import Tick from '#lib/ui/Tick.svelte';
	import { device as stored } from './device.ts';
	import { PROBING, hosts, name, next, ping, signIn, step, type Phase } from './signin.ts';

	const D = 864e5;
	const device = noun();
	const standalone = isStandalone();

	let phase = $state.raw<Phase>(PROBING);
	/** a session this device held, which the server no longer takes */
	let expired = $state(false);
	let list: string[] | null = null;
	let addr = $state<string | null>(null);
	let copied = $state(0);
	let timer: ReturnType<typeof setTimeout> | undefined;
	let gone = false;

	stored.session().then((s) => (expired = !!s && (!!s.ended || s.viewer.expires_at <= Date.now())));

	async function round() {
		clearTimeout(timer);
		if (gone || document.hidden) return;
		try {
			list ??= await hosts();
			addr = list[0] ?? null;
			const via = await ping(list);
			if (!via) phase = step(phase, { kind: 'unanswered' }, Date.now());
			else {
				phase = step(phase, { kind: 'answered', via }, Date.now());
				phase = step(phase, await signIn(via), Date.now());
			}
		} catch {
			phase = step(phase, { kind: 'failed', why: 'no coeus host answered' }, Date.now());
		}
		const at = next(phase, Date.now());
		if (at !== null && !gone) timer = setTimeout(round, Math.max(0, at - Date.now()));
	}

	const again = () => {
		phase = PROBING;
		round();
	};
	const shown = () => {
		if (!document.hidden && phase.kind === 'probing') round();
	};
	document.addEventListener('visibilitychange', shown);
	onDestroy(() => {
		gone = true;
		clearTimeout(timer);
		document.removeEventListener('visibilitychange', shown);
	});
	round();

	async function copy(e: MouseEvent) {
		e.preventDefault();
		if (!addr) return;
		copied = Date.now();
		try {
			await navigator.clipboard.writeText(addr);
		} catch {}
	}

	const checked = $derived(
		phase.kind === 'probing' && phase.checked !== null
			? Math.max(0, Math.floor((now.ms - phase.checked) / 1000))
			: null
	);
</script>

{#if phase.kind === 'signed_in'}
	{@const s = phase.session}
	<div class="auth">
		<div class="auth-in">
			<svg class="check" viewBox="0 0 52 52">
				<circle cx="26" cy="26" r="24" pathLength="1" /><path
					d="M16.5 26.5l6.5 6.5 13-14"
					pathLength="1"
				/>
			</svg>
			<h1>Signed in on the tailnet</h1>
			<p class="sub">
				This {device} can open coeus from anywhere until
				<span class="mono">{day(s.viewer.expires_at)}</span>.
			</p>
			<dl class="kv">
				<div>
					<dt>account</dt>
					<dd>{s.viewer.login.split('@')[0]}</dd>
				</div>
				<div>
					<dt>device</dt>
					<dd>
						{s.viewer.device}
						<span class="m">· {standalone ? 'Home Screen app' : 'browser'}</span>
					</dd>
				</div>
				<div>
					<dt>signed in via</dt>
					<dd>{name(s.via)} <span class="m">· tailnet</span></dd>
				</div>
				<div>
					<dt>session</dt>
					<dd>
						{Math.round((s.viewer.expires_at - s.iat) / D)} days
						<span class="m">· then sign in on the tailnet again</span>
					</dd>
				</div>
			</dl>
			<a
				class="btn tap"
				href="/"
				onclick={(e) => (e.preventDefault(), router.go({ screen: 'fleet' }, true))}
			>
				Open the fleet
			</a>
		</div>
	</div>
{:else}
	<div class="auth">
		<div class="auth-in">
			<span class="wm">coeus</span>
			{#if phase.kind === 'refused'}
				<h1>This device may not sign in</h1>
				<p class="sub">
					{name(phase.via)} answered on the tailnet, but the tailnet policy grants this {device} no access
					to coeus. Once it does, try again.
				</p>
			{:else if expired}
				<h1>Your session ended</h1>
				<p class="sub">
					A session lasts 7 days from the last time this {device} was on the tailnet. Turn on Tailscale
					and coeus signs in again by itself.
				</p>
			{:else}
				<h1>Open this on your tailnet to sign in</h1>
				<p class="sub">
					This address is public, so it cannot start a session. Sign in once from a device on the
					tailnet; the session then works from anywhere for 7 days.
				</p>
			{/if}
			{#if phase.kind !== 'refused'}
				<div class="scan"></div>
				<ol class="steps">
					<li><span class="mono">01</span>Turn on Tailscale on this {device}.</li>
					<li><span class="mono">02</span>Let coeus ask the tailnet address below.</li>
					<li><span class="mono">03</span>It signs in by itself, and works anywhere after.</li>
				</ol>
			{/if}
			<a class="addr tap" href={addr ?? '/signin'} onclick={copy}>
				<span class="mono">{addr ?? 'asking for the tailnet address…'}</span><span class="cp">
					{now.ms - copied < 1600 ? 'copied' : 'copy'}
				</span>
			</a>
			<div class="probe mono">
				{#if phase.kind === 'signing'}
					<Dot s="run" /><span>on the tailnet · signing in through {name(phase.via)}</span>
				{:else if phase.kind === 'refused'}
					<Dot s="bad" /><span>refused by {name(phase.via)} · {phase.why}</span>
				{:else if checked === null}
					<Dot s="wait" /><span>asking the tailnet…</span>
				{:else}
					<Dot s="wait" /><span>
						{phase.failure ?? 'not on the tailnet'} · checked <Tick still v="{checked}s" /> ago
					</span>
				{/if}
			</div>
			{#if phase.kind === 'refused'}
				<a class="btn tap" href="/signin" onclick={(e) => (e.preventDefault(), again())}>
					Try again
				</a>
			{/if}
		</div>
	</div>
{/if}
