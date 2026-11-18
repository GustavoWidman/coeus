import { mount } from 'svelte';
import '#lib/ui/fonts.css';
import '#lib/ui/app.css';
import '#lib/auth/device.ts';
import App from './App.svelte';

mount(App, { target: document.getElementById('app')! });

if (import.meta.env.PROD && 'serviceWorker' in navigator) {
	addEventListener('load', () => {
		navigator.serviceWorker.register('/service-worker.js').then(
			(reg) =>
				document.addEventListener('visibilitychange', () => {
					// a Home Screen app can stay open for days without a launch that would look
					if (!document.hidden) reg.update().catch(() => {});
				}),
			() => {}
		);
	});
}
