/**
 * What the service worker shows for a push, and where a tap on it leads. The fleet sends a
 * Declarative Web Push message, which Safari shows by itself; every other browser hands it to
 * the service worker's `push` handler, which shows the same fields. A browser revokes the
 * permission of a site whose push shows nothing, so a payload that cannot be read still shows
 * a notification.
 */

export interface Notice {
	title: string;
	body: string;
	/** a path of the app, which a tap opens */
	navigate: string;
	tag?: string;
	timestamp?: number;
}

const FALLBACK: Notice = { title: 'coeus', body: '', navigate: '/' };

/** the notification a push carries, from its payload's text */
export function notice(text: string | null, origin: string): Notice {
	let json: unknown;
	try {
		json = JSON.parse(text ?? '');
	} catch {
		return FALLBACK;
	}
	const n = (json as { notification?: Record<string, unknown> } | null)?.notification;
	if (!n || typeof n.title !== 'string' || !n.title) return FALLBACK;
	const str = (v: unknown) => (typeof v === 'string' ? v : undefined);
	return {
		title: n.title,
		body: str(n.body) ?? '',
		navigate: target(n.navigate, origin),
		tag: str(n.tag),
		timestamp: typeof n.timestamp === 'number' ? n.timestamp : undefined
	};
}

/** the path `navigate` names on `origin`, or the fleet for anything else */
export function target(navigate: unknown, origin: string): string {
	if (typeof navigate !== 'string') return '/';
	try {
		const url = new URL(navigate, origin);
		return url.origin === origin ? url.pathname + url.search : '/';
	} catch {
		return '/';
	}
}

/** what the service worker posts to an open window to take it to `path` */
export interface Navigate {
	kind: 'navigate';
	path: string;
}
