/**
 * Whether an element is on screen, as one IntersectionObserver last saw it, so asking costs no
 * layout. Something nobody can see still costs the compositor a layer while it animates, so
 * the motions leave it still. An element counts as off screen until the observer first reports
 * on it, which it starts doing the first time it is asked about.
 */
const observed = new WeakSet<Element>();
const shown = new WeakSet<Element>();

const observer = new IntersectionObserver((entries) => {
	for (const e of entries) {
		if (e.isIntersecting) shown.add(e.target);
		else shown.delete(e.target);
	}
});

export function onScreen(el: Element): boolean {
	if (!observed.has(el)) {
		observed.add(el);
		observer.observe(el);
	}
	return shown.has(el);
}

/** whether a box in viewport coordinates is outside the viewport */
export const offScreen = (r: DOMRect) => r.bottom < 0 || r.top > innerHeight;
