import { describe, expect, test } from 'vitest';
import { card, deployment, finished, host, job, progress, row, T } from '#lib/test/cards.ts';
import { columns, counts, fraction, latest, outcome, share, status, verdictDot } from './status.ts';

const H = 3_600_000;

describe('status', () => {
	test.each([
		[
			'asleep, with when it was last seen',
			card(
				'laptop-mac',
				{},
				{
					kind: 'unreachable',
					since: T - 3 * H,
					reason: { kind: 'offline', last_seen: T - 3 * H }
				}
			),
			{ dot: 'asleep', text: 'asleep · will catch up · seen 3h ago', tone: 'mute' }
		],
		[
			'refusing the read key',
			card(
				'oracle-2',
				{},
				{
					kind: 'unreachable',
					since: T,
					reason: { kind: 'refused', detail: 'this key may not watch' }
				}
			),
			{ dot: 'bad', text: 'refused · this key may not watch', tone: 'bad' }
		],
		[
			'out of reach',
			card(
				'oracle-2',
				{},
				{
					kind: 'unreachable',
					since: T,
					reason: { kind: 'failed', detail: 'connection timed out' }
				}
			),
			{ dot: 'stale', text: 'unreachable · connection timed out', tone: 'warn' }
		],
		[
			'never reached yet',
			{ host: host('lab'), link: { kind: 'connecting' as const }, state: null },
			{ dot: 'wait', text: 'connecting', tone: 'mute' }
		],
		[
			'about to start a queued job',
			card('lab', {
				next: job({
					id: 62,
					state: { kind: 'queued' },
					source: { kind: 'snapshot', label: '3f9a1c0', nar_hash: '' }
				})
			}),
			{ dot: 'wait', text: 'starting #62 switch · snapshot 3f9a1c0' }
		],
		[
			'failed within the hour',
			card('desktop-nixos', {
				recent: [
					finished(88, { kind: 'failed', phase: 'building', message: 'exit 2' }, T - 3 * 60_000)
				]
			}),
			{ dot: 'bad', text: 'failed in building · 3m ago', tone: 'bad' }
		],
		[
			'failed over an hour ago, and running what it ran before',
			card('desktop-nixos', {
				recent: [finished(88, { kind: 'failed', phase: 'building', message: 'exit 2' }, T - 2 * H)]
			}),
			{ dot: 'ok', text: 'failed in building · 2h ago' }
		],
		[
			'held by a paused follow',
			card('altar', {
				follow: { git_ref: 'deployed', latest: null, paused: true },
				deployed: { kind: 'snapshot', label: '3f9a1c0', nar_hash: '' }
			}),
			{ dot: 'warn', text: 'follow paused · snapshot 3f9a1c0', tone: 'warn' }
		],
		[
			'waiting for a reboot',
			card('oracle-1', { reboot_required: true }),
			{ dot: 'ok', text: 'succeeded · reboot required' }
		],
		[
			'waiting for a boot',
			card('oracle-2', { pending_boot: true }),
			{ dot: 'ok', text: 'succeeded · pending boot' }
		],
		['idle', card('altar'), { dot: 'ok', text: 'succeeded · 2h ago' }],
		['idle with no history', card('altar', { recent: [] }), { dot: 'ok', text: 'up to date' }]
	])('%s', (_, c, want) => {
		expect(status(c, undefined, T)).toEqual(want);
	});

	test('counts builds while building, and how far the job is', () => {
		const s = status(card('lab', { running: job() }), progress(), T);
		expect(s).toMatchObject({ dot: 'run', text: 'building', builds: { done: 194, expected: 388 } });
		expect(s.p).toBeCloseTo(0.2 + 0.66 * (0.75 * 0.5 + 0.25 * 0.5));
	});

	test('names the phase, and a catch-up, outside building', () => {
		const s = status(
			card('laptop-mac', {
				running: job({
					state: { kind: 'running', phase: 'evaluating' },
					origin: { kind: 'follow' }
				})
			}),
			undefined,
			T
		);
		expect(s).toMatchObject({ dot: 'run', text: 'catching up · evaluating', builds: undefined });
		expect(s.p).toBeCloseTo(0.07);
	});

	test("ignores another job's progress", () => {
		const s = status(card('lab', { running: job({ id: 62 }) }), progress({ job: 61 }), T);
		expect(s.builds).toBeUndefined();
	});
});

describe('fraction', () => {
	test('is 0 for a job that is not running, and counts no builds before they are known', () => {
		expect(fraction(job({ state: { kind: 'queued' } }), undefined)).toBe(0);
		const unknown = progress({
			builds: { done: 0, expected: 0, running: 0, failed: 0 },
			downloads: { done: 0, expected: 0, running: 0, failed: 0 }
		});
		expect(fraction(job(), unknown)).toBeCloseTo(0.2);
	});

	test('reaches 1 only past verifying', () => {
		expect(
			fraction(job({ state: { kind: 'running', phase: 'verifying' } }), undefined)
		).toBeCloseTo(0.95);
	});
});

test('outcome says how a job ended', () => {
	expect(outcome({ kind: 'up_to_date' })).toBe('up to date');
	expect(outcome({ kind: 'superseded', by: 60 })).toBe('superseded by #60');
	expect(
		outcome({
			kind: 'rolled_back',
			phase: 'verifying',
			message: 'coeusd.service did not become active'
		})
	).toBe('rolled back after verifying: coeusd.service did not become active');
});

describe('deployments', () => {
	const d = deployment({
		waves: [
			{ name: null, hosts: [host('oracle-1')], started_at: T - 400_000, settled_at: T - 300_000 },
			{
				name: null,
				hosts: [host('lab'), host('desktop-nixos')],
				started_at: T - 290_000,
				settled_at: null
			}
		],
		rows: [
			row('oracle-1', { wave: 0, verdict: 'done' }),
			row('desktop-nixos', { wave: 1, verdict: 'failed' }),
			row('lab', { wave: 1, verdict: 'running' }),
			row('altar', { wave: null, verdict: 'pending' }),
			row('laptop-mac', { wave: null, verdict: 'caught_up' })
		]
	});

	test('columns are the waves, then the machines that catch up on their own', () => {
		expect(columns(d).map((c) => [c.name, c.rows.map((r) => r.host), c.ordered, c.done])).toEqual([
			['wave 1', ['oracle-1'], true, true],
			['wave 2', ['desktop-nixos', 'lab'], true, false],
			['catch-up', ['altar', 'laptop-mac'], false, false]
		]);
	});

	test('counts machines by verdict', () => {
		expect(counts(d)).toEqual({ done: 2, run: 1, fail: 1, pend: 1 });
	});

	test("a machine's share follows its running job", () => {
		const lab = card('lab', { running: job() });
		expect(share(row('lab', { verdict: 'running' }), lab, progress())).toBeCloseTo(
			fraction(job(), progress())
		);
		expect(share(row('x', { verdict: 'caught_up' }), undefined, undefined)).toBe(1);
		expect(share(row('x', { verdict: 'pending' }), undefined, undefined)).toBe(0);
	});

	test('a pending machine that sleeps shows asleep', () => {
		const asleep = card(
			'laptop-mac',
			{},
			{ kind: 'unreachable', since: T, reason: { kind: 'offline', last_seen: T } }
		);
		expect(verdictDot('pending', asleep)).toBe('asleep');
		expect(verdictDot('pending', card('altar'))).toBe('wait');
	});

	test('the latest is the newest ordered to more than one machine', () => {
		const one = deployment({ order: 'a' as any, rows: [row('lab'), row('altar', { wave: null })] });
		const two = deployment({ order: 'b' as any, rows: [row('lab'), row('altar')] });
		expect(latest([one, two])?.order).toBe('b');
		expect(latest([one])).toBeUndefined();
	});
});
