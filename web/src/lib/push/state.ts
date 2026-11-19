/**
 * What the notifications control shows, from what the server, the browser and this device
 * say. The server names the subscriptions the fleet sends to by a hash of their endpoint, and
 * the device hashes its own the same way, so it tells "subscribed but not in the list" from
 * "on" without the server learning more than the hash.
 */
import type { PushView } from '#lib/wire/types.ts';

/** `/api/push`'s answer */
export type Server =
	| { kind: 'loading' }
	/** the fleet sends no notifications */
	| { kind: 'none' }
	| { kind: 'failed'; why: string }
	| { kind: 'ok'; push: PushView };

export interface Browser {
	/** the page has `PushManager` */
	push: boolean;
	/** an iPhone or iPad, whose Safari tab has no push but whose Home Screen app does */
	ios: boolean;
	standalone: boolean;
	permission: NotificationPermission;
}

/** the entry of the subscription list that sends to this device, `PushSubscription.toJSON()` and a name */
export interface Entry {
	name: string;
	endpoint: string;
	keys: { p256dh: string; auth: string };
}

/** this device's subscription */
export interface Mine {
	entry: Entry;
	/** the first 16 hex digits of its endpoint's SHA-256 */
	hash: string;
	/** the key it was made with, base64url, or null where the browser does not say */
	key: string | null;
}

/** what happened on this device that the browser does not keep */
export interface Local {
	/** the system prompt is up */
	asking: boolean;
	/** the subscription changed since the list last had it */
	changed: boolean;
	/** the name of the entry the device just stopped, which the list still has */
	removed: string | null;
	/** why the last attempt to subscribe failed */
	failed: string | null;
}

export type Control =
	| { kind: 'loading' }
	| { kind: 'absent' }
	| { kind: 'install' }
	| { kind: 'off'; removed: string | null; failed: string | null }
	| { kind: 'asking' }
	| { kind: 'denied' }
	| { kind: 'uncommitted'; entry: Entry }
	| { kind: 'on' }
	| { kind: 'stale'; entry: Entry }
	/** subscribed under a key the server no longer serves: to subscribe again with its key */
	| { kind: 'rotated' }
	/** subscribed, but the server could not read the list to say whether it holds it */
	| { kind: 'unknown'; why: string };

export const QUIET: Local = { asking: false, changed: false, removed: null, failed: null };

export function control(
	server: Server,
	browser: Browser,
	mine: Mine | null,
	local: Local
): Control {
	if (server.kind === 'loading') return { kind: 'loading' };
	if (server.kind === 'none') return { kind: 'absent' };
	if (!browser.push)
		return browser.ios && !browser.standalone ? { kind: 'install' } : { kind: 'absent' };
	if (browser.permission === 'denied') return { kind: 'denied' };
	if (local.asking) return { kind: 'asking' };
	if (!mine) return { kind: 'off', removed: local.removed, failed: local.failed };
	if (server.kind === 'failed') return { kind: 'unknown', why: server.why };
	if (mine.key !== null && unpad(mine.key) !== unpad(server.push.key)) return { kind: 'rotated' };
	if (server.push.subscribed.includes(mine.hash)) return { kind: 'on' };
	return local.changed
		? { kind: 'stale', entry: mine.entry }
		: { kind: 'uncommitted', entry: mine.entry };
}

const unpad = (b64url: string) => b64url.replace(/=+$/, '');

export function b64url(bytes: ArrayBuffer | Uint8Array): string {
	let s = '';
	for (const b of new Uint8Array(bytes)) s += String.fromCharCode(b);
	return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function bytes(b64url: string): Uint8Array<ArrayBuffer> {
	const s = atob(b64url.replace(/-/g, '+').replace(/_/g, '/'));
	const out = new Uint8Array(s.length);
	for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
	return out;
}

/** the first 16 hex digits of the SHA-256 of `endpoint`, as the server names it */
export async function endpointHash(endpoint: string): Promise<string> {
	const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(endpoint));
	return [...new Uint8Array(digest, 0, 8)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** the entry to add to the list, as the senders read it */
export function entry(name: string, json: PushSubscriptionJSON): Entry | null {
	const { endpoint, keys } = json;
	if (!endpoint || !keys?.p256dh || !keys.auth) return null;
	return { name, endpoint, keys: { p256dh: keys.p256dh, auth: keys.auth } };
}

/** `entry` laid out as it goes into the list's JSON array */
export const pretty = (e: Entry) => JSON.stringify(e, null, 2);
