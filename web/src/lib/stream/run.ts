/**
 * The fleet's lifetime in the page: the stream, which signs each request with this device's
 * session. A refused session sends the page to sign in and waits there until it does.
 */
import { device } from '#lib/auth/device.ts';
import { withFlip } from '#lib/motion/flip.ts';
import { router } from '#lib/router.svelte.ts';
import { connect } from './client.ts';
import { fleet } from './fleet.svelte.ts';

connect('/api/stream', {
	events(events) {
		const apply = () => fleet.apply(events);
		if (fleet.ready && events.some((e) => fleet.structural(e))) withFlip(apply);
		else apply();
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
