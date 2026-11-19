import { mount } from 'svelte';
import '#lib/ui/fonts.css';
import '#lib/ui/app.css';
import '#lib/auth/device.ts';
import { parse, router } from '#lib/router.svelte.ts';
import App from './App.svelte';

mount(App, { target: document.getElementById('app')! });

// a tap on a notification while the app is open takes it to the screen the notification names
navigator.serviceWorker?.addEventListener('message', (event) => {
	const data = event.data as { kind?: unknown; path?: unknown } | null;
	const route = data?.kind === 'navigate' && typeof data.path === 'string' && parse(data.path);
	if (route) router.go(route);
});

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
