/**
 * What shows before the fleet first arrives: nothing at first, so a quick answer never flashes a
 * placeholder, then a skeleton, which once shown stays long enough not to flicker. An answer
 * that failed brings no fleet soon, so the skeleton shows at once.
 */
import { fleet } from '#lib/stream/fleet.svelte.ts';

const QUIET = 250;
const HOLD = 450;

let view = $state<'nothing' | 'skeleton' | 'fleet'>('nothing');
let held = $state(false);

function skeleton() {
	if (view !== 'nothing') return;
	view = 'skeleton';
	setTimeout(() => (held = true), HOLD);
}

setTimeout(skeleton, QUIET);

$effect.root(() => {
	$effect(() => {
		if (fleet.ready && (view === 'nothing' || held)) view = 'fleet';
		else if (fleet.link === 'retrying') skeleton();
	});
});

export const boot = {
	get view() {
		return view;
	},
	/** the fleet took a skeleton's place, and so fades in */
	get faded() {
		return held;
	}
};
