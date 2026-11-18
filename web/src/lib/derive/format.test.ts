import { describe, expect, test } from 'vitest';
import { state, T } from '#lib/test/cards.ts';
import { ago, clk, delta, dur, hms, origin, plat, source } from './format.ts';

describe('format', () => {
	test.each([
		[2_000, 'now'],
		[12_000, '12s ago'],
		[3 * 60_000, '3m ago'],
		[2 * 3_600_000 + 59 * 60_000, '2h ago'],
		[4 * 86_400_000, '4d ago'],
		[-5_000, 'now']
	])('ago %i ms is %s', (d, want) => {
		expect(ago(T - d, T)).toBe(want);
	});

	test('durations and clocks', () => {
		expect(dur(38_400)).toBe('38s');
		expect(dur(252_000)).toBe('4m 12s');
		expect(dur(3_900_000)).toBe('1h 05m');
		expect(clk(272_900)).toBe('4:32');
		expect(clk(-1)).toBe('0:00');
		expect(hms(Date.UTC(2026, 10, 17, 21, 5, 30))).toMatch(/^\d\d:05:30$/);
	});

	test('sources, origins and platforms', () => {
		expect(source({ kind: 'git', rev: '2cbe6da7e56b', git_ref: 'main' })).toBe('main@2cbe6da');
		expect(source({ kind: 'git', rev: '2cbe6da7e56b', git_ref: null })).toBe('2cbe6da');
		expect(source({ kind: 'snapshot', label: '3f9a1c0', nar_hash: '' })).toBe('snapshot 3f9a1c0');
		expect(source({ kind: 'generation', number: 1361 })).toBe('generation 1361');
		expect(
			origin({
				kind: 'github',
				repository: 'o/nix',
				workflow: 'deploy',
				run_id: 1184,
				attempt: 1,
				url: ''
			})
		).toBe('actions · deploy #1184');
		expect(origin({ kind: 'user', user: 'gustavo', host: 'altar' })).toBe('gustavo@altar');
		expect(origin({ kind: 'follow' })).toBe('catch-up');
		expect(plat(state())).toBe('nixos · x86_64');
		expect(plat(state({ platform: 'darwin', system: 'aarch64-darwin' }))).toBe('darwin · arm64');
	});
});

describe('delta', () => {
	test.each([
		[4_215_273, '+4.0'],
		[-50_541_363, '−48.2'],
		[0, '±0.0']
	])('%d bytes reads %s', (bytes, want) => expect(delta(bytes)).toBe(want));
});
