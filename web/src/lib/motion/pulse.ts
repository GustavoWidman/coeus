import { reduced } from './reduced.ts';

const seen = new WeakMap<Element, unknown>();

export const under = (el: Element) => el.closest('[data-under]') !== null;

/** rings the element's `<b>` when `value` changes, but not when it first shows */
export const pulse = (value: unknown) => (el: Element) => {
	const was = seen.get(el);
	seen.set(el, value);
	const b = el.querySelector('b');
	if (was === undefined || was === value || !b || reduced() || under(el)) return;
	b.animate(
		[
			{ transform: 'scale(1)', opacity: 0.6 },
			{ transform: 'scale(3.4)', opacity: 0 }
		],
		{ duration: 520, easing: 'cubic-bezier(.2,.6,.3,1)' }
	);
};

const rolled = new Map<HTMLElement, { at: number; to: string }>();
const ROLL = 170;

/**
 * Writes `to` into `el`, rolling the characters that differ from `from`: each run of changed
 * characters slides up (down when the number fell) and fades, over the run it replaces.
 */
export function roll(el: HTMLElement, from: string | null, to: string) {
	rolled.delete(el);
	if (from === null || reduced() || under(el)) {
		el.textContent = to;
		return;
	}
	rolled.set(el, { at: performance.now(), to });
	const value = (s: string) => parseFloat(s.replace(/[^\d.]/g, ''));
	const down = value(to) < value(from);
	const n = Math.max(from.length, to.length);
	const a = from.padStart(n);
	const b = to.padStart(n);
	el.textContent = '';
	let i = 0;
	while (b[i] === ' ') i++;
	while (i < n) {
		const same = a[i] === b[i];
		let j = i + 1;
		while (j < n && (a[j] === b[j]) === same) j++;
		if (same) el.append(b.slice(i, j));
		else {
			const dg = document.createElement('span');
			dg.className = down ? 'dg d' : 'dg';
			const o = document.createElement('span');
			o.className = 'o';
			o.textContent = a.slice(i, j);
			const nw = document.createElement('span');
			nw.className = 'n';
			nw.textContent = b.slice(i, j);
			dg.append(o, nw);
			el.append(dg);
		}
		i = j;
	}
}

/**
 * Puts plain text back in place of every roll that has ended. The clock calls this as it
 * ticks, when the page lays out anyway, so the spans of a roll never cost a layout of their
 * own and a number that stops changing stops holding them.
 */
export function unroll() {
	const t = performance.now();
	for (const [el, { at, to }] of rolled) {
		if (t - at < ROLL) continue;
		el.textContent = to;
		rolled.delete(el);
	}
}
