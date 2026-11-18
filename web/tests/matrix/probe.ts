/**
 * What the matrix runs in the page before the app: a clock it can hold or run, a count of
 * the stream messages the app receives (with the state's rewrites applied on the way), long
 * tasks, every animation the page starts, and frame intervals while asked for them.
 *
 * Playwright sends the function as source, so it uses nothing from outside its body.
 */
import type { Rewrite } from './states.ts';

export interface ProbeConfig {
	/** epoch milliseconds `Date` reports, held until `run` */
	now: number;
	rewrites: Rewrite[];
}

export interface Seen {
	/** stream messages the app received, all streams together */
	messages: number;
	/** streams opened */
	streams: number;
	longTasks: number[] | null;
	/** `kind|property|where` (a selector or `@keyframes name`) → times started */
	animations: Record<string, number>;
	/** animations started, all kinds together */
	lasting: number;
	frames: number[];
}

declare global {
	interface Window {
		__matrix: {
			seen: Seen;
			run(now: number): void;
			hold(now: number): void;
			frames(on: boolean): void;
		};
	}
}

export function probe(config: ProbeConfig) {
	const seen: Seen = {
		messages: 0,
		streams: 0,
		longTasks: null,
		animations: {},
		lasting: 0,
		frames: []
	};

	// the clock
	const RealDate = Date;
	let base = config.now;
	let since = -1;
	const now = () => (since < 0 ? base : base + performance.now() - since);
	const FakeDate = function (this: unknown, ...args: unknown[]) {
		if (!new.target) return new RealDate(now()).toString();
		return args.length ? new (RealDate as any)(...args) : new RealDate(now());
	} as unknown as DateConstructor;
	FakeDate.now = now;
	FakeDate.parse = RealDate.parse;
	FakeDate.UTC = RealDate.UTC;
	Object.defineProperty(FakeDate, 'prototype', { value: RealDate.prototype });
	globalThis.Date = FakeDate;

	// stream messages, rewritten
	const merge = (into: any, from: any): any => {
		if (typeof from !== 'object' || from === null || Array.isArray(from)) return from;
		const out = { ...(typeof into === 'object' && into !== null ? into : {}) };
		for (const [k, v] of Object.entries(from)) out[k] = merge(out[k], v);
		return out;
	};
	const rewrite = (event: any): any => {
		for (const r of config.rewrites) {
			if ('machine' in r) {
				const card = (m: any) => (m.host === r.machine ? merge(m, r.set) : m);
				if (event.kind === 'snapshot') event.machines = event.machines.map(card);
				if (event.kind === 'machine') event.machine = card(event.machine);
			} else if ('deployments' in r) {
				if (event.kind === 'snapshot') event.deployments = [];
				if (event.kind === 'deployment' || event.kind === 'deployment_removed') return null;
			}
		}
		return event;
	};
	const extraLines = config.rewrites.reduce((n, r) => n + ('lines' in r ? r.lines : 0), 0);
	const filler = () => {
		const kinds = ['build', 'output', 'output', 'fetch'];
		const lines = Array.from({ length: extraLines }, (_, i) => ({
			at: null,
			kind: kinds[i % kinds.length],
			text: `building '/nix/store/${(i * 7919).toString(36).padStart(8, '0')}-package-${i}.drv'`,
			prefix: i % 4 === 1 ? 'rustc-1.84.0' : null
		}));
		return `id: 0\nevent: lines\ndata: ${JSON.stringify({ kind: 'lines', lines, end: 0 })}\n\n`;
	};

	const realFetch = fetch;
	globalThis.fetch = async (input, init) => {
		const response = await realFetch(input, init);
		const type = response.headers.get('content-type') ?? '';
		if (!response.ok || !response.body || !type.startsWith('text/event-stream')) return response;
		seen.streams++;
		const decoder = new TextDecoder();
		const encoder = new TextEncoder();
		let rest = '';
		let filled = extraLines === 0;
		const frame = (text: string): string | null => {
			const data = text
				.split('\n')
				.filter((l) => l.startsWith('data:'))
				.map((l) => l.slice(5).trimStart());
			if (!data.length) return text;
			seen.messages++;
			if (!config.rewrites.length) return text;
			const event = rewrite(JSON.parse(data.join('\n')));
			if (!event) return null;
			const head = text.split('\n').filter((l) => !l.startsWith('data:'));
			let out = [...head, `data: ${JSON.stringify(event)}`].join('\n');
			if (!filled && event.kind === 'started') {
				filled = true;
				out += '\n\n' + filler().trimEnd();
			}
			return out;
		};
		const body = response.body.pipeThrough(
			new TransformStream<Uint8Array, Uint8Array>({
				transform(chunk, out) {
					rest += decoder.decode(chunk, { stream: true });
					const parts = rest.split('\n\n');
					rest = parts.pop()!;
					const kept = parts.map(frame).filter((f) => f !== null);
					if (kept.length) out.enqueue(encoder.encode(kept.join('\n\n') + '\n\n'));
				}
			})
		);
		return new Response(body, {
			status: response.status,
			statusText: response.statusText,
			headers: response.headers
		});
	};

	// long tasks, where the browser reports them
	if (PerformanceObserver.supportedEntryTypes?.includes('longtask')) {
		seen.longTasks = [];
		new PerformanceObserver((list) => {
			for (const e of list.getEntries()) seen.longTasks!.push(Math.round(e.duration));
		}).observe({ type: 'longtask', buffered: true });
	}

	// every animation started: CSS animations and transitions by their events, scripted ones
	// through Element.animate
	// one of 1 ms or less moves nothing: reduced motion shortens every animation to that
	const note = (kind: string, property: string, at: string, duration: number) => {
		if (duration <= 1) return;
		const key = `${kind}|${property.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase())}|${at}`;
		seen.animations[key] = (seen.animations[key] ?? 0) + 1;
		seen.lasting++;
	};
	const IGNORED = ['offset', 'easing', 'composite', 'computedOffset'];
	const properties = (keyframes: Keyframe[]) => [
		...new Set(keyframes.flatMap((k) => Object.keys(k).filter((p) => !IGNORED.includes(p))))
	];
	const duration = (a: Animation | undefined) => Number(a?.effect?.getTiming().duration) || 0;
	const selector = (e: Element) => [e.localName, ...e.classList].join('.');
	addEventListener(
		'animationstart',
		(e) => {
			const a = (e.target as Element)
				.getAnimations()
				.find((x) => (x as CSSAnimation).animationName === e.animationName);
			const keyframes = a?.effect instanceof KeyframeEffect ? a.effect.getKeyframes() : [];
			for (const p of properties(keyframes))
				note('css', p, `@keyframes ${e.animationName}`, duration(a));
		},
		true
	);
	addEventListener(
		'transitionrun',
		(e) => {
			const target = e.target as Element;
			const a = target
				.getAnimations()
				.find((x) => (x as CSSTransition).transitionProperty === e.propertyName);
			note('transition', e.propertyName, selector(target), duration(a));
		},
		true
	);
	const animate = Element.prototype.animate;
	Element.prototype.animate = function (keyframes, options) {
		const a = animate.call(this, keyframes, options);
		const list = Array.isArray(keyframes) ? keyframes : keyframes ? [keyframes] : [];
		for (const p of properties(list as Keyframe[])) note('script', p, selector(this), duration(a));
		return a;
	};

	// frame intervals, only while asked: a frame callback keeps the page drawing
	let framing = false;
	let last = 0;
	const tick = (t: number) => {
		if (!framing) return;
		if (last) seen.frames.push(Math.round((t - last) * 10) / 10);
		last = t;
		requestAnimationFrame(tick);
	};

	window.__matrix = {
		seen,
		run(at) {
			base = at;
			since = performance.now();
		},
		hold(at) {
			base = at;
			since = -1;
		},
		frames(on) {
			framing = on;
			last = 0;
			if (on) requestAnimationFrame(tick);
		}
	};
}
