/**
 * The app's screens as URLs, kept in the browser's history.
 *
 * Each history entry the app writes holds its depth, the number of entries the app pushed
 * before it. When the browser moves through history, the depth it lands on tells a back
 * (lower) from a forward (higher), so the stack can run a pop or a push, and a gesture that
 * already animated the pop can recognize the back it caused. A path that names no screen
 * becomes `/`. "All hosts down" is a state shown over any screen, not a route.
 */

export type Route =
	| { screen: 'fleet' }
	| { screen: 'machine'; host: string }
	| { screen: 'log'; host: string; job: number }
	| { screen: 'deployment'; order: string }
	| { screen: 'signin' };

export type Nav = {
	route: Route;
	depth: number;
	/** How this entry became current: opened, pushed, replaced, or reached through history. */
	kind: 'load' | 'push' | 'replace' | 'back' | 'forward';
	/** The browser already animated the move, as Safari does for a swipe from the edge. */
	animated?: boolean;
};

const routes: [RegExp, (...params: string[]) => Route][] = [
	[/^\/$/, () => ({ screen: 'fleet' })],
	[/^\/machines\/([^/]+)$/, (host) => ({ screen: 'machine', host })],
	[/^\/machines\/([^/]+)\/jobs\/(\d+)$/, (host, id) => ({ screen: 'log', host, job: +id })],
	[/^\/deployments\/([^/]+)$/, (order) => ({ screen: 'deployment', order })],
	[/^\/signin$/, () => ({ screen: 'signin' })]
];

export function parse(path: string): Route | null {
	path = path.replace(/(.)\/$/, '$1');
	for (const [pattern, route] of routes) {
		const m = pattern.exec(path);
		if (!m) continue;
		try {
			return route(...m.slice(1).map(decodeURIComponent));
		} catch {
			return null;
		}
	}
	return null;
}

export function href(route: Route): string {
	const e = encodeURIComponent;
	switch (route.screen) {
		case 'fleet':
			return '/';
		case 'machine':
			return `/machines/${e(route.host)}`;
		case 'log':
			return `/machines/${e(route.host)}/jobs/${route.job}`;
		case 'deployment':
			return `/deployments/${e(route.order)}`;
		case 'signin':
			return '/signin';
	}
}

type Entry = { depth: number };
const depthOf = (state: unknown) => (state as Entry | null)?.depth ?? 0;

function open(): Nav {
	const depth = depthOf(history.state);
	const route = parse(location.pathname);
	history.replaceState({ depth } satisfies Entry, '', route ? undefined : '/');
	return { route: route ?? { screen: 'fleet' }, depth, kind: 'load' };
}

let nav = $state.raw(open());

function go(route: Route, replace = false) {
	const path = href(route);
	if (path === location.pathname) return;
	const depth = nav.depth + (replace ? 0 : 1);
	if (replace) history.replaceState({ depth } satisfies Entry, '', path);
	else history.pushState({ depth } satisfies Entry, '', path);
	nav = { route, depth, kind: replace ? 'replace' : 'push' };
}

addEventListener('popstate', (event) => {
	const depth = depthOf(event.state);
	nav = {
		route: parse(location.pathname) ?? { screen: 'fleet' },
		depth,
		kind: depth < nav.depth ? 'back' : 'forward',
		animated: (event as PopStateEvent & { hasUAVisualTransition?: boolean }).hasUAVisualTransition
	};
});

// A plain click on a link to one of the app's screens navigates without reloading.
addEventListener('click', (event) => {
	if (event.defaultPrevented || event.button !== 0) return;
	if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
	const a = event.target instanceof Element && event.target.closest('a[href]');
	if (!(a instanceof HTMLAnchorElement) || a.target || a.hasAttribute('download')) return;
	const route = a.origin === location.origin && parse(a.pathname);
	if (!route) return;
	event.preventDefault();
	go(route);
});

export const router = {
	get nav() {
		return nav;
	},
	go,
	back: (route: Route) => (nav.depth > 0 ? history.back() : go(route, true))
};
