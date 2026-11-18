/**
 * Keeps coeus opening without the network. Each build precaches itself under its own cache and
 * serves every navigation from the cached shell, so a launch never waits on a host; `/api` is
 * left to the network. A new build installs in the background and waits for the next launch, or
 * for the page to ask it to take over. A navigation with no shell to serve that Cloudflare
 * answers 530, which means no host's connector is up, gets the offline page the build carries,
 * the same page the offline Worker serves.
 */
/// <reference lib="webworker" />
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
