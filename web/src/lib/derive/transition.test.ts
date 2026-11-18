import { describe, expect, test } from 'vitest';
import { card, finished, job, T } from '#lib/test/cards.ts';
import { landed } from './transition.ts';

describe('landed', () => {
	const before = card('lab', {
		running: job({ id: 61 }),
		recent: [finished(60, { kind: 'succeeded' }, T - 1e6)]
	});

	test('names the job that went from running into the history', () => {
		const after = card('lab', {
			recent: [finished(61, { kind: 'succeeded' }, T), finished(60, { kind: 'succeeded' }, T - 1e6)]
		});
		expect(landed(before, after)).toEqual([61]);
	});

	test('names nothing when the history is the same', () => {
		expect(
			landed(before, card('lab', { running: job({ id: 61 }), recent: before.state!.recent }))
		).toEqual([]);
	});

	test('names nothing the first time a machine is seen, or reached', () => {
		expect(landed(undefined, before)).toEqual([]);
		expect(landed({ ...before, state: null }, before)).toEqual([]);
	});

	test('names a job that finished while the history dropped its oldest', () => {
		const full = card('lab', {
			recent: Array.from({ length: 8 }, (_, i) => finished(60 - i, { kind: 'succeeded' }, T - i))
		});
		const next = card('lab', {
			recent: [
				finished(61, { kind: 'failed', phase: 'building', message: 'x' }, T),
				...full.state!.recent.slice(0, 7)
			]
		});
		expect(landed(full, next)).toEqual([61]);
	});
});
