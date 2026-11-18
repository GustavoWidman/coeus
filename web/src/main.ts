import { mount } from 'svelte';
import '#lib/ui/fonts.css';
import '#lib/ui/app.css';
import { withFlip } from '#lib/motion/flip.ts';
import { connect } from '#lib/stream/client.ts';
import { fleet } from '#lib/stream/fleet.svelte.ts';
import App from './App.svelte';

mount(App, { target: document.getElementById('app')! });

connect('/api/stream', {
	events(events) {
		const apply = () => fleet.apply(events);
		if (fleet.ready && events.some((e) => fleet.structural(e))) withFlip(apply);
		else apply();
	},
	link(state) {
		fleet.link = state;
	}
});
