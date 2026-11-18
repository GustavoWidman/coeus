/**
 * A log's lines on the page: only those on screen and half a screen either side, between two
 * paddings that stand for the rest, so a log of twenty thousand lines costs the page what a
 * screenful does. A line drawn once is measured and remembered; one never drawn is placed by
 * an estimate from its length. At the end of the log the view follows new lines, gliding up to
 * them; scrolled up, it counts them instead.
 *
 * The glide is the browser's own smooth scroll, which runs off the main thread. Lines that rose
 * into place while the view jumped to them cost the compositor a frame in every dozen.
 *
 * Each frame reads what the last one drew before it writes, while the layout still stands, so
 * following a log costs one layout per frame that adds lines.
 *
 * The list stands `aria-busy` while lines it was given wait to be drawn or the view glides to
 * them, which is the only way anything outside can tell a log still settling from one at rest.
 */
import { reduced } from '#lib/motion/reduced.ts';
import { onScreen } from '#lib/motion/visible.ts';
import { look, rows, type Lines } from './lines.ts';

/** the line height and top padding of `.logbody`, and the padding around a phase */
const ROW = 18;
const TOP = 10;
const PAD = 8;
/** how far from the end still counts as at the end */
const NEAR = 48;
/**
 * New lines at the end show at most this often: often enough that while a log streams each
 * glide runs into the next, so the compositor never stops between them only to start again a
 * frame late.
 */
const PACE = 150;

export class Tail {
	/** each kept line's height: measured when positive, estimated when negative, unknown at 0 */
	private heights: number[] = [];
	/** drawn lines by their index from the log's start, dropped lines included */
	private drawn = new Map<number, HTMLElement>();
	private fresh = new Set<number>();
	/** the drawn lines, as indices from the log's start */
	private a = 0;
	private b = 0;
	private cols = 80;
	/** the list's width as last observed, and whether the lines must be measured again for it */
	private width = -1;
	private rewrap = true;
	private frame = 0;
	private timer: ReturnType<typeof setTimeout> | undefined;
	private shown = -PACE;
	private added = 0;
	private moved = true;
	private stuck = true;
	/** the view is gliding to the end, which counts as being there until it stops or is stopped */
	private gliding = false;
	private target = 0;
	private unseen = 0;
	private view = 0;
	private scroll = 0;
	private busy = false;
	private resized: ResizeObserver;

	constructor(
		private box: HTMLElement,
		private list: HTMLElement,
		private log: Lines,
		private onUnseen: (n: number) => void
	) {
		box.addEventListener('scroll', this.scrolled, { passive: true });
		for (const e of ['wheel', 'touchstart', 'pointerdown', 'keydown'])
			box.addEventListener(e, this.taken, { passive: true });
		this.resized = new ResizeObserver(([e]) => {
			const width = Math.round(e.contentRect.width);
			if (width !== this.width) this.rewrap = true;
			this.width = width;
			this.moved = true;
			this.schedule();
		});
		this.resized.observe(box);
		// a font that arrives late changes every line's width
		document.fonts.ready.then(() => {
			this.rewrap = true;
			this.schedule();
		});
	}

	destroy() {
		cancelAnimationFrame(this.frame);
		clearTimeout(this.timer);
		this.resized.disconnect();
		this.box.removeEventListener('scroll', this.scrolled);
		for (const e of ['wheel', 'touchstart', 'pointerdown', 'keydown'])
			this.box.removeEventListener(e, this.taken);
	}

	/** the log gained `n` lines at its end, and lost `dropped` from its front */
	grew(n: number, dropped: number) {
		this.heights.splice(0, dropped);
		this.added += n;
		this.mark();
		if (this.frame || this.timer) return;
		// scrolled up, new lines only add to a count. Drawn here rather than in a frame
		// callback, the lines are ready when the browser wakes to show them, not a frame later.
		const wait = this.stuck ? this.shown + PACE - performance.now() : 0;
		this.timer = setTimeout(
			() => {
				this.timer = undefined;
				if (!this.frame) this.draw();
			},
			Math.max(0, wait)
		);
	}

	/** to the end of the log */
	jump() {
		const { box } = this;
		this.see(0);
		this.stuck = true;
		if (reduced()) {
			box.scrollTop = 1e9;
			this.gliding = false;
			this.mark();
			return;
		}
		const end = box.scrollHeight - box.clientHeight;
		// a long way is mostly skipped, so the smooth part only shows that it moved
		if (end - box.scrollTop > 2 * box.clientHeight) box.scrollTop = end - box.clientHeight;
		this.glide(end);
		this.mark();
	}

	private glide(to: number) {
		// a scroll to where the view already stands sends no scroll event to end the glide
		if (Math.abs(this.box.scrollTop - to) < 1) return;
		this.box.scrollTo({ top: to, behavior: 'smooth' });
		this.target = to;
		this.gliding = true;
	}

	private mark() {
		const busy = this.added > 0 || this.gliding;
		if (busy === this.busy) return;
		this.busy = busy;
		this.box.setAttribute('aria-busy', String(busy));
	}

	private see(n: number) {
		if (n === this.unseen) return;
		this.unseen = n;
		this.onUnseen(n);
	}

	/** a person moving the log stops a glide */
	private taken = () => {
		this.gliding = false;
		this.mark();
	};

	private scrolled = () => {
		this.moved = true;
		this.schedule();
	};

	private schedule() {
		if (!this.frame)
			this.frame = requestAnimationFrame(() => {
				this.frame = 0;
				this.draw();
			});
	}

	private draw() {
		this.layout();
		this.mark();
	}

	private layout() {
		const { box, list, log } = this;
		const gone = log.gone;
		const n = log.all.length;

		// reads. After a scroll the layout is usually clean: the browser laid the page out to
		// scroll it. Lines drawn while following were measured when the last frame laid out.
		const rewrapped = this.rewrap;
		if (rewrapped) {
			this.rewrap = false;
			this.cols = this.columns();
			this.heights.fill(0);
			for (const k of this.drawn.keys()) this.fresh.add(k);
		}
		if (this.fresh.size) this.measure();
		if (this.moved) {
			this.moved = false;
			this.view = box.clientHeight;
			this.scroll = box.scrollTop;
			let left = box.scrollHeight - this.scroll - this.view;
			if (this.gliding && (left < 1 || Math.abs(this.scroll - this.target) < 1)) {
				this.gliding = false;
				// lines measured on the way down moved the end past where the glide was going
				if (left >= 1) {
					box.scrollTop = 1e9;
					this.scroll = box.scrollTop;
					left = 0;
				}
			}
			this.stuck = this.gliding || left < NEAR;
			if (this.stuck) this.see(0);
		}
		const { view, scroll } = this;

		const added = this.added;
		this.added = 0;
		if (added) {
			clearTimeout(this.timer);
			this.timer = undefined;
			this.shown = performance.now();
		}
		if (added && !this.stuck) this.see(this.unseen + added);

		const h = this.heights;
		while (h.length < n) h.push(0);
		for (let i = 0; i < n; i++) if (h[i] === 0) h[i] = -this.estimate(i);

		// which lines to draw, as indices into the kept lines
		let na = n;
		let nb = n;
		const ai = this.a - gone;
		const bi = this.b - gone;
		if (this.stuck) {
			// from wherever the view stands, which a glide leaves above the end for a moment;
			// what is drawn stays while it reaches a quarter screen above that and not much more,
			// so a glide moves nothing on the page
			const end = this.sum(0, n);
			// a glide is never longer than a screen, so the view's top is within two of the end
			const top = Math.max(end - 2 * view, Math.min(scroll - TOP, end - view));
			const at = this.drawn.size && ai >= 0 ? this.sum(0, ai) : -Infinity;
			if (at <= top - view / 4 && at >= top - view * 1.5) na = ai;
			else na = this.around(top - view / 2, end)[0];
		} else {
			const top = scroll - TOP;
			const covered =
				this.drawn.size > 0 &&
				ai >= 0 &&
				(ai === 0 || this.sum(0, ai) <= top - view / 4) &&
				(bi >= n || this.sum(0, bi) >= top + view * 1.25);
			if (covered) [na, nb] = [ai, bi];
			else [na, nb] = this.around(top - view / 2, top + view * 1.5);
		}
		const same = na === ai && nb === bi && (this.drawn.size > 0 || n === 0);
		if (same && !added && !rewrapped) return;

		// writes
		const above = ai > na ? this.sum(na, Math.min(ai, nb)) : 0;
		const prepended = this.place(gone + na, gone + nb);
		list.style.paddingTop = `${this.sum(0, na)}px`;
		list.style.paddingBottom = `${this.sum(nb, n)}px`;
		if (this.stuck) {
			// the layout these force is the frame's own, so measuring after it is free
			const glide =
				added > 0 && n > added && this.sum(n - added, n) < view && !reduced() && onScreen(box);
			if (glide) this.glide(box.scrollHeight - view);
			else if (added || !this.gliding) {
				box.scrollTop = 1e9;
				this.gliding = false;
			}
			this.measure();
		} else if (prepended) {
			// lines drawn above the screen take their real heights; what is on screen stays put
			this.measure();
			box.scrollTop = scroll + this.sum(na, na + prepended) - above;
		}
	}

	private sum(from: number, to: number) {
		let y = 0;
		for (let i = Math.max(0, from); i < Math.min(to, this.heights.length); i++)
			y += Math.abs(this.heights[i]);
		return y;
	}

	/** the kept lines that reach into [lo, hi) */
	private around(lo: number, hi: number): [number, number] {
		const h = this.heights;
		let i = 0;
		let y = 0;
		while (i < h.length && y + Math.abs(h[i]) <= lo) y += Math.abs(h[i++]);
		const a = i;
		while (i < h.length && y < hi) y += Math.abs(h[i++]);
		return [a, i];
	}

	/**
	 * Draws the lines from `a` to `b` (indices from the log's start), keeping those already
	 * drawn, and answers how many it drew above those it kept.
	 */
	private place(a: number, b: number): number {
		const { list, log } = this;
		for (const [k, el] of this.drawn)
			if (k < a || k >= b) {
				el.remove();
				this.drawn.delete(k);
				this.fresh.delete(k);
			}
		if (!this.drawn.size) this.a = this.b = a;
		const gone = log.gone;
		const before = document.createDocumentFragment();
		for (let k = a; k < this.a; k++) before.append(this.line(k, gone));
		list.prepend(before);
		const after = document.createDocumentFragment();
		for (let k = Math.max(this.b, a); k < b; k++) after.append(this.line(k, gone));
		list.append(after);
		const prepended = this.a - a;
		this.a = a;
		this.b = b;
		return prepended;
	}

	private line(k: number, gone: number) {
		const i = k - gone;
		const { cls, runs } = look(this.log.all[i], this.log.all[i - 1]);
		const el = document.createElement('div');
		el.className = cls ? `ln ${cls}` : 'ln';
		const no = document.createElement('span');
		no.className = 'no';
		no.textContent = String(k + 1);
		const tx = document.createElement('span');
		tx.className = 'tx';
		for (const r of runs) {
			if (!r.dim) tx.append(r.text);
			else {
				const s = document.createElement('span');
				s.className = r.dim;
				s.textContent = r.text;
				tx.append(s);
			}
		}
		el.append(no, tx);
		this.drawn.set(k, el);
		this.fresh.add(k);
		return el;
	}

	private measure() {
		const gone = this.log.gone;
		for (const k of this.fresh) {
			const el = this.drawn.get(k);
			if (el && k >= gone) this.heights[k - gone] = el.offsetHeight || ROW;
		}
		this.fresh.clear();
	}

	private estimate(i: number) {
		const line = this.log.all[i];
		const { cls, runs } = look(line, this.log.all[i - 1]);
		const pad = line.kind !== 'phase' ? 0 : cls.endsWith('pp') ? PAD : 2 * PAD;
		return rows(runs.map((r) => r.text).join(''), this.cols) * ROW + pad;
	}

	/** how many characters a line's text column holds */
	private columns() {
		const probe = document.createElement('div');
		probe.className = 'ln';
		probe.innerHTML = '<span class="no"></span><span class="tx"></span>';
		const ruler = document.createElement('span');
		ruler.style.cssText = 'display:inline-block;white-space:pre';
		ruler.textContent = '0'.repeat(64);
		probe.lastChild!.appendChild(ruler);
		this.list.prepend(probe);
		const room = (probe.lastChild as HTMLElement).clientWidth;
		const glyph = ruler.getBoundingClientRect().width / 64;
		probe.remove();
		return glyph > 0 ? Math.max(1, Math.floor(room / glyph + 1e-3)) : 80;
	}
}
