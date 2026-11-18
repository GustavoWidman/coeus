<!--
	Only the top two screens are ever drawn, and the one under the top only while a push, a pop
	or a swipe from the left edge uncovers it.
-->
<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { router, type Nav, type Route } from '#lib/router.svelte.ts';
	import { E, stage } from '#lib/motion/flip.ts';
	import { reduced } from '#lib/motion/reduced.ts';
	import Screen from './Screen.svelte';
	import type SheetView from './Sheet.svelte';

	interface Entry {
		key: string;
		route: Route;
	}

	const FLEET: Entry = { key: 'fleet', route: { screen: 'fleet' } };
	const entry = (r: Route): Entry =>
		r.screen === 'machine' || r.screen === 'log'
			? { key: `m:${r.host}`, route: { screen: 'machine', host: r.host } }
			: r.screen === 'deployment'
				? { key: `d:${r.order}`, route: r }
				: FLEET;
	const build = (r: Route) => {
		const e = entry(r);
		return e === FLEET ? [FLEET] : [FLEET, e];
	};

	let screens = $state.raw<Entry[]>(build(router.nav.route));
	let root: HTMLElement;
	let mounted = false;
	/** counts moves, so a move another one overtook leaves the screens alone when it ends */
	let moves = 0;
	let running: Animation[] = [];
	/** the screen a pop is sliding away, already gone from the history */
	let leaving: Entry | null = null;

	const el = (i: number) => root.children[i] as HTMLElement;
	const shade = (e: HTMLElement) => e.lastElementChild as HTMLElement;

	function tw(e: HTMLElement, keyframes: Keyframe[], ms: number) {
		Object.assign(e.style, keyframes.at(-1));
		if (reduced()) return;
		const a = e.animate(keyframes, { duration: ms, easing: E });
		running.push(a);
		return a.finished.catch(() => {});
	}

	function settle() {
		if (!mounted) return;
		running = [];
		const n = screens.length;
		for (let i = 0; i < n; i++) {
			const e = el(i);
			const top = i === n - 1;
			e.style.transform = top ? '' : 'translateX(-28%)';
			e.style.visibility = top ? '' : 'hidden';
			e.inert = !top;
			e.toggleAttribute('data-under', !top);
			shade(e).style.opacity = top ? '' : '0.06';
		}
		stage.roots = [el(n - 1)];
		stage.busy = false;
	}

	function uncover(e: HTMLElement) {
		e.style.visibility = '';
		e.inert = false;
		e.removeAttribute('data-under');
	}

	/** ends whatever move is under way where it was going, so the next starts from rest */
	function begin() {
		for (const a of running) a.finish();
		running = [];
		if (leaving) screens = screens.filter((s) => s !== leaving);
		leaving = null;
		stage.busy = true;
		return ++moves;
	}

	async function push(e: Entry) {
		const move = begin();
		screens = [...screens, e];
		await tick();
		const n = screens.length;
		const prev = el(n - 2);
		uncover(prev);
		await Promise.all([
			tw(el(n - 1), [{ transform: 'translateX(100%)' }, { transform: 'translateX(0%)' }], 240),
			tw(prev, [{ transform: 'translateX(0%)' }, { transform: 'translateX(-28%)' }], 240),
			tw(shade(prev), [{ opacity: 0 }, { opacity: 0.06 }], 240)
		]);
		if (move === moves) settle();
	}

	/** slides the top screen off from `from` px, or drops it at once if the browser slid it */
	async function pop(from = 0, slide = true) {
		const move = begin();
		const n = screens.length;
		const prev = el(n - 2);
		leaving = screens[n - 1];
		uncover(prev);
		if (slide) {
			const k = 1 - from / innerWidth;
			const ms = 90 + 150 * k;
			await Promise.all([
				tw(
					el(n - 1),
					[{ transform: `translateX(${from}px)` }, { transform: 'translateX(100%)' }],
					ms
				),
				tw(prev, [{ transform: `translateX(${-28 * k}%)` }, { transform: 'translateX(0%)' }], ms),
				tw(shade(prev), [{ opacity: 0.06 * k }, { opacity: 0 }], ms)
			]);
		}
		if (move !== moves) return;
		leaving = null;
		screens = screens.slice(0, -1);
		await tick();
		if (move === moves) settle();
	}

	function follow(nav: Nav) {
		if (leaving) {
			const move = begin();
			tick().then(() => move === moves && settle());
		}
		const e = entry(nav.route);
		const n = screens.length;
		if (e.key === screens[n - 1].key) return;
		if (n > 1 && e.key === screens[n - 2].key) return void pop(0, !nav.animated);
		if (nav.kind === 'push' || nav.kind === 'forward') return void push(e);
		const move = begin();
		screens = build(nav.route);
		tick().then(() => move === moves && settle());
	}

	$effect(() => {
		const nav = router.nav;
		untrack(() => follow(nav));
	});

	// the sheet loads with the first log opened, and stays to put that one and the next away
	let Sheet = $state.raw<typeof SheetView | null>(null);
	/** a sheet stands fully risen over the screens */
	let covered = $state(false);
	$effect(() => {
		if (router.nav.route.screen === 'log' && !Sheet)
			import('./Sheet.svelte').then((m) => (Sheet = m.default));
	});

	onMount(() => {
		mounted = true;
		settle();
		return () => {
			mounted = false;
			stage.roots = [];
		};
	});

	const back = () => {
		const r = router.nav.route;
		if (r.screen === 'log') router.back({ screen: 'machine', host: r.host });
		else if (screens.length > 1) router.back(screens[screens.length - 2].route);
	};

	// in a Safari tab the left edge belongs to Safari
	const standalone = () =>
		matchMedia('(display-mode: standalone)').matches ||
		(navigator as { standalone?: boolean }).standalone === true;

	let swipe: { x0: number; t0: number; d: number } | null = null;
	let swiped = false;

	function down(e: PointerEvent) {
		if (swipe || stage.busy || screens.length < 2 || e.clientX >= 24 || !standalone()) return;
		begin();
		swipe = { x0: e.clientX, t0: e.timeStamp, d: 0 };
		uncover(el(screens.length - 2));
		root.setPointerCapture(e.pointerId);
	}

	function move(e: PointerEvent) {
		if (!swipe) return;
		const n = screens.length;
		const k = 1 - (swipe.d = Math.max(0, e.clientX - swipe.x0)) / innerWidth;
		el(n - 1).style.transform = `translateX(${swipe.d}px)`;
		el(n - 2).style.transform = `translateX(${-28 * k}%)`;
		shade(el(n - 2)).style.opacity = String(0.06 * k);
	}

	async function up(e: PointerEvent) {
		if (!swipe) return;
		const { d, t0 } = swipe;
		swipe = null;
		if (d > 6) {
			swiped = true;
			setTimeout(() => (swiped = false));
		}
		const fast = d >= 24 && d / Math.max(1, e.timeStamp - t0) > 0.5;
		if (e.type === 'pointerup' && (d > innerWidth * 0.33 || fast)) {
			const to = screens[screens.length - 2].route;
			await pop(d);
			router.back(to);
			return;
		}
		const move = moves;
		const n = screens.length;
		const prev = el(n - 2);
		await Promise.all([
			tw(el(n - 1), [{ transform: `translateX(${d}px)` }, { transform: 'translateX(0px)' }], 180),
			tw(
				prev,
				[{ transform: getComputedStyle(prev).transform }, { transform: 'translateX(-28%)' }],
				180
			)
		]);
		if (move === moves) settle();
	}

	function gestures(surface: HTMLElement) {
		const click = (e: MouseEvent) => swiped && (e.preventDefault(), e.stopPropagation());
		surface.addEventListener('pointerdown', down);
		surface.addEventListener('pointermove', move);
		surface.addEventListener('pointerup', up);
		surface.addEventListener('pointercancel', up);
		surface.addEventListener('click', click, true);
	}
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && back()} />

<div
	class="stage"
	aria-busy={router.nav.route.screen === 'log' && !covered}
	bind:this={root}
	{@attach gestures}
>
	{#each screens as s (s.key)}
		<Screen route={s.route} onback={back} />
	{/each}
</div>
{#if Sheet}<Sheet stage={() => root} oncover={(on) => (covered = on)} />{/if}
