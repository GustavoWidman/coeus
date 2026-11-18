/**
 * The fleet's lifetime in the page: the kept fleet first, then the stream, which signs each
 * request with this device's session. A refused session sends the page to sign in and waits there until it does; a
 * session past a day old renews itself in the background whenever the page shows.
 */
import { device, due } from '#lib/auth/device.ts';
import { withFlip } from '#lib/motion/flip.ts';
import { router } from '#lib/router.svelte.ts';
import { connect } from './client.ts';
import { fleet } from './fleet.svelte.ts';
import { keep, load } from './kept.ts';

const changed = keep(() => (fleet.cached ? null : fleet.snapshot()));

load().then((kept) => kept && fleet.restore(kept.snapshot));

connect('/api/stream', {
	events(events) {
		const apply = () => fleet.apply(events);
		if (fleet.ready && events.some((e) => fleet.structural(e))) withFlip(apply);
		else apply();
		changed();
	},
	link(state) {
		fleet.link = state;
	},
	sign: (path) => device.headers('GET', path),
	async refused() {
		await device.end();
		router.go({ screen: 'signin' }, true);
		await device.signedIn();
	}
});

async function renew() {
	const session = await device.session();
	if (session && due(session, Date.now())) (await import('#lib/auth/signin.ts')).renew();
}

renew();
document.addEventListener('visibilitychange', () => document.hidden || renew());
