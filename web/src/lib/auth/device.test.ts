import { describe, expect, test } from 'vitest';
import {
	RENEW_AFTER,
	b64url,
	claims,
	due,
	proof,
	requestMessage,
	signature,
	signinMessage,
	type Session
} from './device.ts';

const P256 = { name: 'ECDSA', namedCurve: 'P-256' } as const;
const SHA256 = { name: 'ECDSA', hash: 'SHA-256' } as const;
const bytes = (s: string) =>
	Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
const text = (s: string) => new TextEncoder().encode(s);
const keys = () =>
	crypto.subtle.generateKey(P256, false, ['sign', 'verify']) as Promise<CryptoKeyPair>;

const T = 1_794_927_600_000;
const token = (iat: number, exp: number) =>
	`v1.${b64url(text(JSON.stringify({ v: 1, kid: '00000000', iat, exp })))}.mac`;
const session = (over: Partial<Session> = {}): Session => ({
	token: token(T, T + 7 * 864e5),
	viewer: { login: 'gustavo@github', device: 'iphone', expires_at: T + 7 * 864e5 },
	via: 'https://lab.tail4a3ea.ts.net:45730',
	iat: T,
	...over
});

describe('proof', () => {
	test('signs the method, path, moment and token hash with the device key', async () => {
		const pair = await keys();
		const s = session();
		const headers = await proof(s, pair.privateKey, 'GET', '/api/stream', T);
		expect(headers.authorization).toBe(`Coeus ${s.token}`);
		const [ts, sig] = headers['coeus-proof'].split('.');
		expect(Number(ts)).toBe(T);
		expect(bytes(sig)).toHaveLength(64);
		const hash = b64url(await crypto.subtle.digest('SHA-256', text(s.token)));
		const signed = requestMessage('GET', '/api/stream', T, hash);
		expect(signed).toBe(`coeus-req-v1\nGET\n/api/stream\n${T}\n${hash}`);
		expect(await crypto.subtle.verify(SHA256, pair.publicKey, bytes(sig), text(signed))).toBe(true);
		const other = requestMessage('GET', '/api/hello', T, hash);
		expect(await crypto.subtle.verify(SHA256, pair.publicKey, bytes(sig), text(other))).toBe(false);
	});

	test('signs a sign-in over the public origin and the moment', async () => {
		const pair = await keys();
		const message = signinMessage('https://coeus.r3dlust.com', T);
		expect(message).toBe(`coeus-signin-v1\nhttps://coeus.r3dlust.com\n${T}`);
		const sig = await signature(pair.privateKey, message);
		expect(await crypto.subtle.verify(SHA256, pair.publicKey, bytes(sig), text(message))).toBe(
			true
		);
	});
});

describe('claims', () => {
	test('reads when a token was issued and when it ends', () => {
		expect(claims(token(T, T + 1))).toEqual({ iat: T, exp: T + 1 });
	});

	test('reads nothing from a token that is not one', () => {
		expect(claims('nonsense')).toBeNull();
		expect(claims('v1.!!!.mac')).toBeNull();
	});
});

describe('due', () => {
	test('renews a session past a day old that still lasts', () => {
		expect(due(session(), T + RENEW_AFTER)).toBe(false);
		expect(due(session(), T + RENEW_AFTER + 1)).toBe(true);
	});

	test('leaves an ended or expired session to signing in', () => {
		expect(due(session({ ended: true }), T + 2 * RENEW_AFTER)).toBe(false);
		expect(due(session(), T + 7 * 864e5)).toBe(false);
	});
});
