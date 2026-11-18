<!--
	A job's log on a phone: a sheet over the screens, which sink back under it. It follows the
	address: it rises when the address names a log and falls when the address moves on, which
	Done, a drag down from its header, Escape and the browser's back all do.
-->
<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { router } from '#lib/router.svelte.ts';
	import { E } from '#lib/motion/flip.ts';
	import { reduced } from '#lib/motion/reduced.ts';
	import type LogView from './Log.svelte';

	let { stage }: { stage: () => HTMLElement } = $props();

	type Shown = { host: string; id: number };
	const SUNK = 'translateY(8px) scale(.94)';

	let shown = $state.raw<Shown | null>(null);
	let Log = $state.raw<typeof LogView | null>(null);
	let wrap = $state<HTMLElement>();
	let moves = 0;
	let running: Animation[] = [];
	/** the sheet is on its way down */
	let falling = false;
	/** a drag already sent the sheet down, so the address moving on only finishes it */
	let dropped: Promise<unknown> | null = null;
	let drag: { y0: number; t0: number; d: number } | null = null;

	const app = () => document.getElementById('app')!;
	const backdrop = () => wrap!.firstElementChild as HTMLElement;
	const sheet = () => wrap!.lastElementChild as HTMLElement;

	function tw(e: HTMLElement, keyframes: Keyframe[], ms: number) {
		Object.assign(e.style, keyframes.at(-1));
		if (reduced()) return;
		const a = e.animate(keyframes, { duration: ms, easing: E });
		running.push(a);
		return a.finished.catch(() => {});
	}

	function begin() {
		if (shown && wrap) cover(false);
		for (const a of running) a.finish();
		running = [];
		return ++moves;
	}

	function sink(on: boolean) {
		const s = stage();
		s.inert = on;
		s.toggleAttribute('data-under', on);
		app().classList.toggle('sheeted', on);
		if (!on) {
			s.style.transform = '';
			s.removeAttribute('data-covered');
		}
	}

	async function open(to: Shown) {
		if (shown && !falling) {
			shown = to;
			return;
		}
		// ends a fall under way, whose close then stands down
		const move = begin();
		falling = false;
		dropped = null;
		Log ??= (await import('./Log.svelte')).default;
		if (move !== moves) return;
		shown = to;
		await tick();
		sink(true);
		await Promise.all([
			tw(sheet(), [{ transform: 'translateY(100%)' }, { transform: 'translateY(0px)' }], 260),
			tw(backdrop(), [{ opacity: 0 }, { opacity: 1 }], 260),
			tw(stage(), [{ transform: 'none' }, { transform: SUNK }], 260)
		]);
		if (move === moves) cover(true);
	}

	/** the sheet stands fully risen over the screens, or is about to move */
	function cover(on: boolean) {
		stage().toggleAttribute('data-covered', on);
		if (on) sheet().style.transform = '';
	}

	/** slides the sheet down from wherever it stands */
	function fall(ms: number) {
		const s = sheet();
		const at = getComputedStyle(s).transform;
		return Promise.all([
			tw(
				s,
				[
					{ transform: at === 'none' ? 'translateY(0px)' : at },
					{ transform: `translateY(${s.getBoundingClientRect().height}px)` }
				],
				ms
			),
			tw(backdrop(), [{ opacity: getComputedStyle(backdrop()).opacity }, { opacity: 0 }], ms),
			tw(stage(), [{ transform: getComputedStyle(stage()).transform }, { transform: 'none' }], ms)
		]);
	}

	async function close() {
		const fell = dropped;
		const move = fell ? moves : begin();
		falling = true;
		await (fell ?? fall(200));
		if (move !== moves) return;
		falling = false;
		dropped = null;
		shown = null;
		sink(false);
	}

	$effect(() => {
		const r = router.nav.route;
		untrack(() => {
			if (r.screen === 'log') open({ host: r.host, id: r.job });
			else if (shown) close();
		});
	});

	const leave = () => shown && router.back({ screen: 'machine', host: shown.host });

	function down(e: PointerEvent) {
		const t = e.target as Element;
		if (drag || falling || !t.closest('.logh') || t.closest('a, button')) return;
		begin();
		drag = { y0: e.clientY, t0: e.timeStamp, d: 0 };
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
	}

	function move(e: PointerEvent) {
		if (!drag) return;
		const d = (drag.d = Math.max(0, e.clientY - drag.y0));
		const k = d / sheet().getBoundingClientRect().height;
		sheet().style.transform = `translateY(${d}px)`;
		backdrop().style.opacity = String(1 - k);
		stage().style.transform = `translateY(${8 * (1 - k)}px) scale(${0.94 + 0.06 * k})`;
	}

	function up(e: PointerEvent) {
		if (!drag) return;
		const { d, t0 } = drag;
		drag = null;
		const fast = d >= 24 && d / Math.max(1, e.timeStamp - t0) > 0.6;
		if (e.type === 'pointerup' && (d > 110 || fast)) {
			begin();
			falling = true;
			dropped = fall(200);
			leave();
			return;
		}
		tw(sheet(), [{ transform: `translateY(${d}px)` }, { transform: 'translateY(0px)' }], 180);
		tw(backdrop(), [{ opacity: getComputedStyle(backdrop()).opacity }, { opacity: 1 }], 180);
		tw(stage(), [{ transform: getComputedStyle(stage()).transform }, { transform: SUNK }], 180);
	}
</script>

{#if shown && Log}
	<div class="sheet-wrap" bind:this={wrap}>
		<div class="backdrop"></div>
		<div
			class="sheet"
			role="dialog"
			tabindex="-1"
			aria-label="job log"
			onpointerdown={down}
			onpointermove={move}
			onpointerup={up}
			onpointercancel={up}
		>
			<span class="grab"></span>
			<div class="sbody">
				{#key `${shown.host}/${shown.id}`}
					<Log host={shown.host} id={shown.id} onclose={leave} />
				{/key}
			</div>
		</div>
	</div>
{/if}
