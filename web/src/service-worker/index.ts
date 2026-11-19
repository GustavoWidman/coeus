/**
 * Keeps coeus opening without the network. Each build precaches itself under its own cache and
 * serves every navigation from the cached shell, so a launch never waits on a host; `/api` is
 * left to the network. A new build installs in the background and waits for the next launch, or
 * for the page to ask it to take over. A navigation with no shell to serve that Cloudflare
 * answers 530, which means no host's connector is up, gets the offline page the build carries,
 * the same page the offline Worker serves.
 *
 * It also shows the fleet's notifications where the browser does not show them by itself, and
 * a tap on one opens the screen it names, in the window already open if there is one.
 */
/// <reference lib="webworker" />
import { markChanged } from './changed.ts';
import { notice, target, type Navigate } from './notice.ts';

const sw = self as unknown as ServiceWorkerGlobalScope;

// the build writes these in once it knows every file it made
const BUILD = '__COEUS_BUILD__';
const FILES: string[] = JSON.parse('__COEUS_FILES__');

const CACHE = `coeus-${BUILD}`;
const SHELL = '/';
const OFFLINE = '/offline.html';
const PRECACHE = [SHELL, ...FILES];
const precached = new Set(PRECACHE);

sw.addEventListener('install', (event) => {
	event.waitUntil(caches.open(CACHE).then((cache) => cache.addAll(PRECACHE)));
});

sw.addEventListener('activate', (event) => {
	const old = (key: string) => key.startsWith('coeus-') && key !== CACHE;
	event.waitUntil(
		caches.keys().then((keys) => Promise.all(keys.filter(old).map((k) => caches.delete(k))))
	);
});

sw.addEventListener('message', (event) => {
	if (event.data === 'take-over') void sw.skipWaiting();
});

sw.addEventListener('fetch', (event) => {
	const { request } = event;
	if (request.method !== 'GET') return;
	const url = new URL(request.url);
	if (url.origin !== sw.location.origin || url.pathname.startsWith('/api/')) return;
	if (request.mode === 'navigate') event.respondWith(navigate(request));
	else if (precached.has(url.pathname)) event.respondWith(fromCache(url.pathname, request));
});

async function navigate(request: Request): Promise<Response> {
	const cache = await caches.open(CACHE);
	const shell = await cache.match(SHELL);
	if (shell) return shell;
	try {
		const res = await fetch(request);
		if (res.status !== 530) return res;
	} catch {}
	const page = await cache.match(OFFLINE);
	if (!page) return Response.error();
	return new Response(page.body, {
		status: 503,
		headers: {
			'content-type': 'text/html; charset=utf-8',
			'cache-control': 'no-store',
			'coeus-offline': '1'
		}
	});
}

async function fromCache(key: string, request: Request): Promise<Response> {
	const cache = await caches.open(CACHE);
	return (await cache.match(key)) ?? fetch(request);
}

sw.addEventListener('push', (event) => {
	const n = notice(event.data?.text() ?? null, sw.location.origin);
	event.waitUntil(
		sw.registration.showNotification(n.title, {
			body: n.body,
			tag: n.tag,
			icon: '/icons/icon-192.png',
			data: { navigate: n.navigate },
			...(n.timestamp === undefined ? {} : { timestamp: n.timestamp })
		} as NotificationOptions)
	);
});

sw.addEventListener('notificationclick', (event) => {
	event.notification.close();
	const data = event.notification.data as { navigate?: unknown } | null;
	event.waitUntil(open(target(data?.navigate, sw.location.origin)));
});

/** focuses a window of the app and takes it to `path`, or opens one there */
async function open(path: string) {
	const windows = await sw.clients.matchAll({ type: 'window', includeUncontrolled: true });
	const mine = windows.find((w) => new URL(w.url).origin === sw.location.origin);
	if (!mine) return void (await sw.clients.openWindow(path));
	mine.postMessage({ kind: 'navigate', path } satisfies Navigate);
	await mine.focus().catch(() => {});
}

// the browser replaced the subscription, so the list holds one that no longer reaches this
// device: the app shows the new one to put in its place
sw.addEventListener('pushsubscriptionchange', (event) => {
	const e = event as Event & {
		waitUntil(p: Promise<unknown>): void;
		oldSubscription?: PushSubscription | null;
		newSubscription?: PushSubscription | null;
	};
	e.waitUntil(
		(async () => {
			const options = e.oldSubscription?.options;
			if (!e.newSubscription && options?.applicationServerKey)
				await sw.registration.pushManager
					.subscribe({ userVisibleOnly: true, applicationServerKey: options.applicationServerKey })
					.catch(() => {});
			await markChanged().catch(() => {});
		})()
	);
});
