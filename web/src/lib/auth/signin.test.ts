import { afterEach, describe, expect, test, vi } from 'vitest';
import { PERIOD, PROBING, next, ping, step, type Phase } from './signin.ts';

const T = 1_794_927_600_000;
const LAB = 'https://lab.tail4a3ea.ts.net:45730';
const ORACLE = 'https://oracle-1.tail4a3ea.ts.net:45730';

describe('step', () => {
	test('asks again a period after an unanswered round', () => {
		const p = step(PROBING, { kind: 'unanswered' }, T);
		expect(p).toEqual({ kind: 'probing', checked: T, failure: null });
		expect(next(PROBING, T)).toBe(T);
		expect(next(p, T + 1000)).toBe(T + PERIOD);
	});

	test('signs in through the host that answered, then stops asking', () => {
		const signing = step(PROBING, { kind: 'answered', via: LAB }, T);
		expect(signing).toEqual({ kind: 'signing', via: LAB });
		expect(next(signing, T)).toBeNull();
		const session = {
			token: 't',
			viewer: { login: 'gustavo@github', device: 'iphone', expires_at: T + 7 * 864e5 },
			via: LAB,
			iat: T
		};
		const done = step(signing, { kind: 'signed_in', session }, T);
		expect(done).toEqual({ kind: 'signed_in', session });
		expect(next(done, T)).toBeNull();
	});

	test('waits for the person after a refusal, and asks again after a failure', () => {
		const signing: Phase = { kind: 'signing', via: LAB };
		const refused = step(signing, { kind: 'refused', via: LAB, why: 'no grant' }, T);
		expect(next(refused, T)).toBeNull();
		const failed = step(signing, { kind: 'failed', why: 'tailscale did not answer' }, T);
		expect(failed).toEqual({ kind: 'probing', checked: T, failure: 'tailscale did not answer' });
		expect(next(failed, T)).toBe(T + PERIOD);
	});
});

describe('ping', () => {
	afterEach(() => vi.unstubAllGlobals());

	test('takes the first host that answers', async () => {
		vi.stubGlobal('fetch', async (url: string) => {
			if (url.startsWith(LAB)) throw new TypeError('unreachable');
			return new Response(null, { status: 204 });
		});
		expect(await ping([LAB, ORACLE])).toBe(ORACLE);
	});

	test('gives up on a host that does not answer in time', async () => {
		const never = (_: string, init: RequestInit) =>
			new Promise((_, fail) => init.signal!.addEventListener('abort', () => fail(new Error())));
		vi.stubGlobal('fetch', never);
		const started = Date.now();
		expect(await ping([LAB], 50)).toBeNull();
		expect(Date.now() - started).toBeLessThan(1000);
	});

	test('counts an error status as no answer, and no hosts as none', async () => {
		vi.stubGlobal('fetch', async () => new Response(null, { status: 502 }));
		expect(await ping([LAB])).toBeNull();
		expect(await ping([])).toBeNull();
	});
});
