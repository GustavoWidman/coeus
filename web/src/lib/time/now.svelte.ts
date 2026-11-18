/** The one clock the screens read: it moves once a second, and not at all while the page is hidden. */
import { unroll } from '#lib/motion/pulse.ts';

let ms = $state(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;

function start() {
	clearInterval(timer);
	ms = Date.now();
	if (!document.hidden)
		timer = setInterval(() => {
			unroll();
			ms = Date.now();
		}, 1000);
}

document.addEventListener('visibilitychange', start);
start();

export const now = {
	get ms() {
		return ms;
	}
};
