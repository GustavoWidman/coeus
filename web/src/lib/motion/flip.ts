/**
 * Things moving because the fleet changed. `withFlip` measures every `[data-key]` element on
 * screen, applies the change, and slides each one that moved from where it was, less what its
 * keyed parent already slides. A keyed element leaving fades out pinned where it was (`ghost`),
 * and one arriving in a list that was already on screen drops in (`enter`).
 */
import { tick } from 'svelte';
import { under } from './pulse.ts';
import { reduced } from './reduced.ts';

const CURVE = [0.23, 1, 0.32, 1] as const;
export const E = `cubic-bezier(${CURVE.join(',')})`;

// Svelte's transitions take their easing as a function
const ease = (x: number) => {
	const [x1, y1, x2, y2] = CURVE;
	const at = (t: number, a: number, b: number) =>
		3 * a * t * (1 - t) ** 2 + 3 * b * t ** 2 * (1 - t) + t ** 3;
	let lo = 0;
	let hi = 1;
	for (let i = 0; i < 20; i++) {
		const mid = (lo + hi) / 2;
		if (at(mid, x1, x2) < x) lo = mid;
		else hi = mid;
	}
	return at((lo + hi) / 2, y1, y2);
};

export const stage = {
	roots: [] as Element[],
	busy: false
};

let before: Map<Element, DOMRect> | null = null;

export async function withFlip(change: () => void) {
	if (reduced() || stage.busy || document.hidden || !stage.roots.length) return change();
	const keyed = stage.roots.flatMap((r) => [...r.querySelectorAll('[data-key]')]);
	const was = new Map(keyed.map((e) => [e, e.getBoundingClientRect()]));
	before = was;
	change();
	await tick();
	before = null;

	const moved = new Map<Element, number>();
	for (const [e, r] of was) {
		if (!e.isConnected) continue;
		const dy = r.top - e.getBoundingClientRect().top;
		if (Math.abs(dy) > 0.5) moved.set(e, dy);
	}
	for (const [e, dy] of moved) {
		let along = 0;
		for (let p = e.parentElement; p; p = p.parentElement) {
			if (moved.has(p)) {
				along = moved.get(p)!;
				break;
			}
		}
		const d = dy - along;
		if (Math.abs(d) <= 0.5) continue;
		// above what stands still, so the browser composites the sliding element alone
		const el = e as HTMLElement;
		el.style.zIndex = '1';
		el.animate([{ transform: `translateY(${d}px)` }, { transform: 'none' }], {
			duration: 220,
			easing: E
		}).onfinish = () => (el.style.zIndex = '');
	}
}

const PINNED = [
	'position',
	'left',
	'top',
	'width',
	'height',
	'margin',
	'pointer-events',
	'z-index',
	'overflow'
];

/** `on` false: the element comes and goes with its list instead of on its own */
export function ghost(node: HTMLElement, on = true) {
	const r = before?.get(node) ?? node.getBoundingClientRect();
	if (!on || reduced() || stage.busy || under(node)) return { duration: 0 };
	if (r.height === 0 || r.bottom < 0 || r.top > innerHeight) return { duration: 0 };
	Object.assign(node.style, {
		position: 'fixed',
		left: r.left + 'px',
		top: r.top + 'px',
		width: r.width + 'px',
		height: r.height + 'px',
		margin: '0',
		pointerEvents: 'none',
		zIndex: '3',
		overflow: 'hidden'
	});
	return {
		duration: 160,
		easing: ease,
		css: (t: number) => `opacity:${t};transform:translateY(${(t - 1) * 4}px)`
	};
}

export function enter(node: HTMLElement, on = true) {
	// an element that comes back while it fades out is still pinned where it stood
	for (const p of PINNED) node.style.removeProperty(p);
	if (!on || reduced() || under(node)) return { duration: 0 };
	return {
		duration: 220,
		easing: ease,
		css: (t: number) => `opacity:${t};transform:translateY(${(t - 1) * 6}px)`
	};
}
