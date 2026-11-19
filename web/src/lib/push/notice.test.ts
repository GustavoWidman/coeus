import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';
import { notice, target } from '../../service-worker/notice.ts';

const ORIGIN = 'https://coeus.example.com';
/** a deploy's summary exactly as `coeus deploy` sends it, pinned by the sender's own test */
const SENT = readFileSync(new URL('../../../../contract/push.json', import.meta.url), 'utf8');

describe('notice', () => {
	test('shows what the fleet sends, as it sends it', () => {
		expect(notice(SENT, ORIGIN)).toEqual({
			title: 'Deployed caf4e2b · 2 of 3',
			body: 'chore(flake): bump nixpkgs\n2 switched in 6m40s\npending: laptop-mac (asleep)',
			navigate: '/deployments/9f3c0a1b2c3d4e5f',
			tag: 'deploy/switch/caf4e2b9a1d6',
			timestamp: 1795038000000
		});
	});

	test('shows the fields of a declarative message, its link as a path', () => {
		const text = JSON.stringify({
			web_push: 8030,
			notification: {
				title: 'Deployed 3f2a9c1 · 4 of 6',
				body: 'chore(flake): bump nixpkgs',
				navigate: `${ORIGIN}/deployments/9f3c0a1b2c3d4e5f`,
				tag: 'deploy/switch/3f2a9c1d0e4b',
				timestamp: 1795038000000,
				lang: 'en',
				dir: 'ltr'
			}
		});
		expect(notice(text, ORIGIN)).toEqual({
			title: 'Deployed 3f2a9c1 · 4 of 6',
			body: 'chore(flake): bump nixpkgs',
			navigate: '/deployments/9f3c0a1b2c3d4e5f',
			tag: 'deploy/switch/3f2a9c1d0e4b',
			timestamp: 1795038000000
		});
	});

	test('shows something for anything it cannot read', () => {
		const fallback = { title: 'coeus', body: '', navigate: '/' };
		for (const text of [null, '', 'not json', 'null', '{}', '{"notification":{"body":"x"}}'])
			expect(notice(text, ORIGIN), String(text)).toEqual(fallback);
	});

	test('leaves out members of the wrong type', () => {
		const text = JSON.stringify({ notification: { title: 't', body: 3, tag: {}, timestamp: 'x' } });
		expect(notice(text, ORIGIN)).toEqual({
			title: 't',
			body: '',
			navigate: '/',
			tag: undefined,
			timestamp: undefined
		});
	});
});

describe('target', () => {
	test('is a path of the app, and the fleet for anything that leaves it', () => {
		expect(target(`${ORIGIN}/machines/lab/jobs/62`, ORIGIN)).toBe('/machines/lab/jobs/62');
		expect(target('/deployments/abc', ORIGIN)).toBe('/deployments/abc');
		expect(target('https://evil.example/machines/lab', ORIGIN)).toBe('/');
		expect(target('javascript:alert(1)', ORIGIN)).toBe('/');
		expect(target(undefined, ORIGIN)).toBe('/');
	});
});
