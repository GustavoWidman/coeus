import { describe, expect, test } from 'vitest';
import {
	QUIET,
	b64url,
	bytes,
	control,
	endpointHash,
	entry,
	pretty,
	type Browser,
	type Mine,
	type Server
} from './state.ts';

const KEY =
	'BBgqG0VbQAPnBlmSzWD_uRkfTJJ7rNkfZc3HbxlUfhd8DBhqYt_DqIxDFK71gxU01Umj-1CqpRCOP0NQP_4183g';
const OTHER =
	'BA1MWds1hwEBqXBwJs858BCz2aokDANojOcDnn2RKAF5Nrm3n4dfeOldxakyl-dkpJKNGN7MFrjh3_yENprVJdI';
const IPHONE = 'https://web.push.apple.com/QGxfixture-iphone';

const server = (subscribed: string[] = ['4029f3734d796cf1']): Server => ({
	kind: 'ok',
	push: { key: KEY, subscribed, edit: null }
});
const phone: Browser = { push: true, ios: true, standalone: true, permission: 'granted' };
const mine = (over: Partial<Mine> = {}): Mine => ({
	entry: { name: 'iphone', endpoint: IPHONE, keys: { p256dh: 'p', auth: 'a' } },
	hash: '4029f3734d796cf1',
	key: KEY,
	...over
});

describe('endpointHash', () => {
	test('is the first 16 hex digits of the SHA-256, as the server computes it', async () => {
		// the same vectors as the server's own test (src/web/test/push.rs)
		expect(await endpointHash(IPHONE)).toBe('4029f3734d796cf1');
		expect(await endpointHash('https://fcm.googleapis.com/fcm/send/abc')).toBe('4e9bdab8bbe7189c');
	});
});

describe('control', () => {
	test('shows nothing until the server answers, and nothing where the fleet sends none', () => {
		expect(control({ kind: 'loading' }, phone, mine(), QUIET).kind).toBe('loading');
		expect(control({ kind: 'none' }, phone, mine(), QUIET).kind).toBe('absent');
	});

	test('asks for the Home Screen in an iPhone tab, and says nothing where push cannot be', () => {
		const tab = { ...phone, push: false, standalone: false };
		expect(control(server(), tab, null, QUIET).kind).toBe('install');
		expect(control(server(), { ...tab, ios: false }, null, QUIET).kind).toBe('absent');
		expect(control(server(), { ...tab, standalone: true }, null, QUIET).kind).toBe('absent');
	});

	test('a refusal in the settings comes before anything this device did', () => {
		const denied = { ...phone, permission: 'denied' as const };
		expect(control(server(), denied, mine(), { ...QUIET, asking: true }).kind).toBe('denied');
	});

	test('off until subscribed, asking while the prompt is up', () => {
		const fresh = { ...phone, permission: 'default' as const };
		expect(control(server(), fresh, null, QUIET)).toEqual({
			kind: 'off',
			removed: null,
			failed: null
		});
		expect(control(server(), fresh, null, { ...QUIET, asking: true }).kind).toBe('asking');
		expect(control(server(), phone, null, { ...QUIET, removed: 'iphone' })).toEqual({
			kind: 'off',
			removed: 'iphone',
			failed: null
		});
	});

	test('on once the list holds the subscription, until then the entry to add', () => {
		expect(control(server(), phone, mine(), QUIET).kind).toBe('on');
		expect(control(server([]), phone, mine(), QUIET)).toEqual({
			kind: 'uncommitted',
			entry: mine().entry
		});
		// a changed subscription is on as soon as the list has its new endpoint
		expect(control(server(), phone, mine(), { ...QUIET, changed: true }).kind).toBe('on');
	});

	test('a subscription that changed since the list had it shows as changed', () => {
		const renewed = mine({ hash: 'ffffffffffffffff' });
		expect(control(server(), phone, renewed, { ...QUIET, changed: true })).toEqual({
			kind: 'stale',
			entry: renewed.entry
		});
	});

	test('a subscription made under another key subscribes again', () => {
		expect(control(server(), phone, mine({ key: OTHER }), QUIET).kind).toBe('rotated');
		expect(control(server(), phone, mine({ key: `${KEY}=` }), QUIET).kind).toBe('on');
		expect(control(server(), phone, mine({ key: null }), QUIET).kind).toBe('on');
	});

	test('a list the server cannot read says so, rather than calling the device unlisted', () => {
		const failed: Server = { kind: 'failed', why: 'the server answered 503' };
		expect(control(failed, phone, mine(), QUIET)).toEqual({
			kind: 'unknown',
			why: 'the server answered 503'
		});
		expect(control(failed, phone, null, QUIET).kind).toBe('off');
	});
});

describe('entry', () => {
	test('is the subscription as the list holds it, named, without its expiry', () => {
		const e = entry('iphone', {
			endpoint: IPHONE,
			expirationTime: null,
			keys: { p256dh: 'BNc', auth: 'q1d' }
		});
		expect(pretty(e!)).toBe(
			[
				'{',
				'  "name": "iphone",',
				`  "endpoint": "${IPHONE}",`,
				'  "keys": {',
				'    "p256dh": "BNc",',
				'    "auth": "q1d"',
				'  }',
				'}'
			].join('\n')
		);
		expect(entry('iphone', { endpoint: IPHONE, keys: {} })).toBeNull();
	});

	test('keys travel as base64url both ways', () => {
		const raw = bytes(KEY);
		expect(raw.length).toBe(65);
		expect(raw[0]).toBe(4);
		expect(b64url(raw)).toBe(KEY);
	});
});
