/**
 * Signing in from inside the app. The public origin cannot start a session, so the app asks
 * the serving hosts' tailnet addresses, which only answer on the tailnet: it pings each every
 * few seconds, and once one answers it sends that host its public key, signed by the private
 * one. The host names the caller from its tailnet address and returns a session bound to the
 * key. A session past a day old renews the same way, unseen, whenever the device is on the
 * tailnet as the app comes to the screen.
 */
import { device, due, signature, signinMessage, type Session } from './device.ts';
import type { Hello, Ms, SignedIn } from '#lib/wire/types.ts';

/** how often the sign-in screen asks, and how long it waits for an answer */
export const PERIOD = 3000;
export const TIMEOUT = 1500;

export type Phase =
	/** no address answered; `checked` is when it last asked */
	| { kind: 'probing'; checked: Ms | null; failure: string | null }
	| { kind: 'signing'; via: string }
	| { kind: 'signed_in'; session: Session }
	/** a host answered but the tailnet policy keeps this device out */
	| { kind: 'refused'; via: string; why: string };

export type Outcome =
	| { kind: 'unanswered' }
	| { kind: 'answered'; via: string }
	| { kind: 'signed_in'; session: Session }
	| { kind: 'refused'; via: string; why: string }
	/** an answer that is no verdict on this device, so asking again may go better */
	| { kind: 'failed'; why: string };

export const PROBING: Phase = { kind: 'probing', checked: null, failure: null };

export function step(phase: Phase, outcome: Outcome, now: Ms): Phase {
	switch (outcome.kind) {
		case 'unanswered':
			return { kind: 'probing', checked: now, failure: null };
		case 'answered':
			return { kind: 'signing', via: outcome.via };
		case 'failed':
			return { kind: 'probing', checked: now, failure: outcome.why };
		case 'signed_in':
		case 'refused':
			return outcome;
	}
}

/** when to ask again, or never: a refusal waits for the person, a session ends the asking */
export function next(phase: Phase, now: Ms): Ms | null {
	if (phase.kind !== 'probing') return null;
	return phase.checked === null ? now : phase.checked + PERIOD;
}

/** the first of `hosts` whose `/auth/ping` answers within `timeout`, if any does */
export async function ping(hosts: string[], timeout = TIMEOUT): Promise<string | null> {
	if (!hosts.length) return null;
	try {
		return await Promise.any(
			hosts.map(async (host) => {
				const res = await fetch(`${host}/auth/ping`, {
					cache: 'no-store',
					signal: AbortSignal.timeout(timeout)
				});
				if (!res.ok) throw new Error(`${res.status}`);
				return host;
			})
		);
	} catch {
		return null;
	}
}

/** the serving hosts' sign-in addresses, which the public origin names without a session */
export async function hosts(): Promise<string[]> {
	const res = await fetch('/api/hello', { cache: 'no-store' });
	if (!res.ok) throw new Error(`${res.status}`);
	return ((await res.json()) as Hello).signin;
}

/** asks `via` for a session bound to this device's key */
export async function signIn(via: string): Promise<Outcome> {
	const keys = await device.keys();
	const key = await crypto.subtle.exportKey('jwk', keys.publicKey);
	const ts = Date.now();
	const sig = await signature(keys.privateKey, signinMessage(location.origin, ts));
	let res: Response;
	try {
		res = await fetch(`${via}/auth/session`, {
			method: 'POST',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify({ key: { kty: key.kty, crv: key.crv, x: key.x, y: key.y }, ts, sig }),
			cache: 'no-store',
			signal: AbortSignal.timeout(10_000)
		});
	} catch {
		return { kind: 'failed', why: `${name(via)} stopped answering` };
	}
	const why = res.ok ? '' : (await res.text().catch(() => '')) || `${res.status}`;
	if (res.status === 403) return { kind: 'refused', via, why };
	if (!res.ok) return { kind: 'failed', why };
	const { token, viewer } = (await res.json()) as SignedIn;
	const session: Session = { token, viewer, via, iat: Date.now() };
	await device.keep(session);
	return { kind: 'signed_in', session };
}

/** one round: find a host that answers, and sign in through it */
export async function attempt(): Promise<Outcome> {
	let list: string[];
	try {
		list = await hosts();
	} catch {
		return { kind: 'failed', why: 'no coeus host answered' };
	}
	const via = await ping(list);
	return via ? signIn(via) : { kind: 'unanswered' };
}

/** renews the session, without a word, if it is due and a sign-in host answers */
export async function renew() {
	const session = await device.session();
	if (session && due(session, Date.now())) await attempt();
}

/** a sign-in address as its machine's name */
export const name = (via: string) => {
	try {
		return new URL(via).hostname.split('.')[0];
	} catch {
		return via;
	}
};
