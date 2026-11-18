import { describe, expect, test } from 'vitest';
import type { LogLine } from '#lib/wire/types.ts';
import { KEEP, Lines, look, rows } from './lines.ts';

const line = (text: string, more: Partial<LogLine> = {}): LogLine => ({
	at: null,
	kind: 'note',
	text,
	prefix: null,
	...more
});

describe('Lines', () => {
	test('keeps every line up to its limit, and past it drops the oldest in a batch', () => {
		const log = new Lines();
		expect(log.add(Array.from({ length: KEEP }, (_, i) => line(`${i}`)))).toBe(0);
		expect(log.add([line('one more')])).toBe(0);
		const dropped = log.add(Array.from({ length: 2_000 }, (_, i) => line(`late ${i}`)));
		expect(dropped).toBe(2_001);
		expect(log.all.length).toBe(KEEP);
		expect(log.gone).toBe(2_001);
		expect(log.all[0].text).toBe('2001');
	});
});

describe('look', () => {
	test('cuts store hashes and the cache address short, and dims them', () => {
		const { cls, runs } = look(
			line(
				"copying path '/nix/store/z7h7badbzq9f9hhlir3p0jg67gwyiziq-cargo-1.84.0' from 'https://cache.nixos.org'",
				{ kind: 'fetch' }
			)
		);
		expect(cls).toBe('c');
		expect(runs).toEqual([
			{ text: "copying path '" },
			{ text: '/nix/store/z7h7bad…-', dim: 'hs' },
			{ text: "cargo-1.84.0'" },
			{ text: ' from cache.nixos.org', dim: 'hs' }
		]);
	});

	test('names the builder before what it printed', () => {
		expect(look(line('installing', { kind: 'output', prefix: 'go-1.23.4' })).runs).toEqual([
			{ text: 'go-1.23.4>', dim: 'px' },
			{ text: ' installing' }
		]);
	});

	test("says which lines are coeus's own, and marks phases as headings", () => {
		const phase = line('building', { kind: 'phase', at: 1 });
		expect(look(phase)).toEqual({ cls: 'p', runs: [{ text: '── building' }] });
		expect(look(phase, phase).cls).toBe('p pp');
		expect(look(line('took /var/lib/coeus/job.lock', { at: 1 })).runs).toEqual([
			{ text: 'coeus: took /var/lib/coeus/job.lock' }
		]);
		expect(look(line('succeeded', { kind: 'done', at: 1 })).cls).toBe('k');
		expect(look(line('error: builder failed', { kind: 'error' })).cls).toBe('e');
	});
});

describe('rows', () => {
	test.each([
		['', 10, 1],
		['0123456789', 10, 1],
		['aaaa bbbb cccc', 10, 2],
		['aaaa bbbb ', 9, 1],
		['01234567890123456789012', 10, 3],
		['nixos-system-lab', 8, 3],
		['ab 0123456789abcdefghij', 10, 3]
	])('%j in %d columns takes %d rows', (text, cols, want) => {
		expect(rows(text, cols)).toBe(want);
	});
});
