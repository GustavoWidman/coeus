/**
 * This device's key pair and session. The private key is made non-extractable and kept in
 * IndexedDB (`coeus-device`), which stores the key itself rather than its bytes, so nothing the
 * page runs can copy it out; the session token lives beside it. Every request to `/api` carries
 * the token and a fresh signature by the key over the request, which the server checks against
 * the public key the token names.
 */
import { store } from '#lib/idb.ts';
import type { Ms, Viewer } from '#lib/wire/types.ts';

export interface Session {
	token: string;
	viewer: Viewer;
	/** the sign-in address that issued it */
	via: string;
	/** when it was issued, always a moment on the tailnet */
	iat: Ms;
	/** the server refused it before `viewer.expires_at` */
	ended?: boolean;
}

interface Stored {
	keys: CryptoKeyPair;
	session: Session | null;
}

/** a session older than this renews when the app shows and a sign-in host answers */
export const RENEW_AFTER = 24 * 60 * 60 * 1000;

/** whether a session is due a renewal at `now`, which only works while it lasts */
export const due = (session: Session, now: Ms) =>
	!session.ended && now - session.iat > RENEW_AFTER && now < session.viewer.expires_at;

const P256 = { name: 'ECDSA', namedCurve: 'P-256' } as const;
const SHA256 = { name: 'ECDSA', hash: 'SHA-256' } as const;

export const b64url = (bytes: ArrayBuffer | Uint8Array) => {
	let s = '';
	for (const b of new Uint8Array(bytes)) s += String.fromCharCode(b);
	return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
};

const text = (s: string) => new TextEncoder().encode(s);

/** what a token says about itself; its MAC is the server's to check */
export function claims(token: string): { iat: Ms; exp: Ms } | null {
	const part = token.split('.')[1];
	if (!part) return null;
	try {
		const json = JSON.parse(atob(part.replace(/-/g, '+').replace(/_/g, '/')));
		return typeof json.iat === 'number' && typeof json.exp === 'number'
			? { iat: json.iat, exp: json.exp }
			: null;
	} catch {
		return null;
	}
}

/** what a request's `Coeus-Proof` signs */
export const requestMessage = (method: string, path: string, ts: Ms, tokenHash: string) =>
	`coeus-req-v1\n${method}\n${path}\n${ts}\n${tokenHash}`;

/** what signing in signs */
export const signinMessage = (origin: string, ts: Ms) => `coeus-signin-v1\n${origin}\n${ts}`;

/** `r || s` over `message`, base64url, as the server reads it */
export async function signature(key: CryptoKey, message: string) {
	return b64url(await crypto.subtle.sign(SHA256, key, text(message)));
}

/** the headers that let a request to `path` (with its query) in */
export async function proof(
	session: Session,
	key: CryptoKey,
	method: string,
	path: string,
	ts: Ms
) {
	const hash = b64url(await crypto.subtle.digest('SHA-256', text(session.token)));
	const sig = await signature(key, requestMessage(method, path, ts, hash));
	return { authorization: `Coeus ${session.token}`, 'coeus-proof': `${ts}.${sig}` };
}

const db = store('coeus-device');
const KEY = 'this';

async function read(): Promise<Stored | null> {
	try {
		return (await db.get<Stored>(KEY)) ?? null;
	} catch {
		return null;
	}
}

const write = (stored: Stored) => db.put(KEY, stored);

/** read once when the page starts, then kept in step with every write */
let current: Promise<Stored | null> = read();
let waiting: (() => void)[] = [];

export const device = {
	async session(): Promise<Session | null> {
		return (await current)?.session ?? null;
	},

	/** this device's key pair, made the first time it is asked for */
	async keys(): Promise<CryptoKeyPair> {
		const stored = await current;
		if (stored) return stored.keys;
		const keys = (await crypto.subtle.generateKey(P256, false, [
			'sign',
			'verify'
		])) as CryptoKeyPair;
		const made = { keys, session: null };
		current = Promise.resolve(made);
		await write(made);
		return keys;
	},

	async keep(session: Session) {
		const next = { keys: await this.keys(), session };
		current = Promise.resolve(next);
		await write(next);
		for (const ok of waiting.splice(0)) ok();
	},

	/** resolves once a session is kept */
	signedIn: () => new Promise<void>((ok) => waiting.push(ok)),

	/** marks the session as one the server no longer takes; the key pair stays */
	async end() {
		const stored = await current;
		if (!stored?.session || stored.session.ended) return;
		const next = { ...stored, session: { ...stored.session, ended: true } };
		current = Promise.resolve(next);
		await write(next);
	},

	/** the headers a request to `path` needs, or none before signing in */
	async headers(method: string, path: string): Promise<Record<string, string>> {
		const stored = await current;
		if (!stored?.session || stored.session.ended) return {};
		return proof(stored.session, stored.keys.privateKey, method, path, Date.now());
	}
};
