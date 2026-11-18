import { describe, expect, test } from 'vitest';
import { card, deployment, finished, host, job, progress, T } from '#lib/test/cards.ts';
import type { FleetEvent, Hostname, OrderId } from '#lib/wire/types.ts';
import { Fleet } from './fleet.svelte.ts';

const snapshot = (over: Partial<Extract<FleetEvent, { kind: 'snapshot' }>> = {}): FleetEvent => ({
	kind: 'snapshot',
	schema: 1,
	build: '',
	served_by: host('lab'),
	viewer: { login: 'gustavo@github', device: 'iphone', expires_at: T },
	machines: [card('altar'), card('lab', { running: job() })],
	progress: [progress()],
	deployments: [deployment()],
	...over
});

describe('Fleet', () => {
	test('is ready once a snapshot arrives', () => {
		const f = new Fleet();
		expect(f.ready).toBe(false);
		f.apply([snapshot()]);
		expect(f.ready).toBe(true);
		expect(f.machines.map((m) => m.host)).toEqual(['altar', 'lab']);
	});

	test('replaces a machine in place and notes the jobs that reached its history', () => {
		const f = new Fleet();
		f.apply([snapshot()]);
		const altar = f.machine('altar');
		f.apply([
			{
				kind: 'machine',
				machine: card('lab', {
					recent: [finished(61, { kind: 'succeeded' }, T), ...card('lab').state!.recent]
				})
			}
		]);
		expect(f.machines.map((m) => m.host)).toEqual(['altar', 'lab']);
		expect(f.machine('altar')).toBe(altar);
		expect(f.landed.map((l) => [l.host, l.job])).toEqual([['lab', 61]]);
	});

	test('tells what moves things on screen from numbers changing in place', () => {
		const f = new Fleet();
		f.apply([snapshot()]);
		const numbers = progress({ builds: { done: 200, expected: 388, running: 4, failed: 0 } });
		expect(f.structural({ kind: 'progress', progress: numbers })).toBe(false);
		const churn = progress({ active: ['rustc-1.84.0'] });
		expect(f.structural({ kind: 'progress', progress: churn })).toBe(true);
		expect(f.structural({ kind: 'progress', progress: progress({ job: 62 }) })).toBe(true);
		expect(f.structural({ kind: 'machine', machine: card('altar') })).toBe(true);
	});

	test('beats for a change to a machine or a deployment, not for a build at work', () => {
		const f = new Fleet();
		f.apply([snapshot()]);
		const beat = f.beat;
		f.apply([{ kind: 'progress', progress: progress({ active: ['rustc-1.84.0'] }) }]);
		expect(f.beat).toBe(beat);
		f.apply([{ kind: 'machine', machine: card('altar', { pending_boot: true }) }]);
		expect(f.beat).toBe(beat + 1);
		f.apply([{ kind: 'deployment', deployment: deployment({ settled_at: T }) }]);
		expect(f.beat).toBe(beat + 2);
	});

	test('clears progress, and keeps deployments newest first', () => {
		const f = new Fleet();
		f.apply([snapshot()]);
		f.apply([
			{ kind: 'progress_cleared', host: 'lab' as Hostname },
			{ kind: 'deployment', deployment: deployment({ order: 'new' as OrderId, ordered_at: T }) },
			{
				kind: 'deployment',
				deployment: deployment({ order: 'old' as OrderId, ordered_at: T - 1e9 })
			}
		]);
		expect(f.progress[host('lab')]).toBeUndefined();
		expect(f.deployments.map((d) => d.order)).toEqual(['new', '2d2eef4459f7ebbf', 'old']);
		f.apply([{ kind: 'deployment_removed', order: 'new' as OrderId }]);
		expect(f.deployments.map((d) => d.order)).toEqual(['2d2eef4459f7ebbf', 'old']);
	});
});

describe('Fleet while hosts are down', () => {
	test('shows a kept snapshot until the stream sends its own', () => {
		const f = new Fleet();
		const kept = snapshot({ served_by: host('oracle-1') }) as Extract<
			FleetEvent,
			{ kind: 'snapshot' }
		>;
		f.restore(kept, T - 60_000);
		expect(f.ready).toBe(true);
		expect(f.cached).toBe(true);
		expect(f.asOf).toBe(T - 60_000);
		expect(f.snapshot()?.served_by).toBe('oracle-1');
		f.setLink('live');
		f.apply([snapshot()]);
		expect(f.cached).toBe(false);
		expect(f.asOf).toBeNull();
		expect(f.servedBy).toBe('lab');
	});

	test('a kept snapshot never replaces what the stream sent', () => {
		const f = new Fleet();
		f.apply([snapshot()]);
		f.restore(
			snapshot({ served_by: host('oracle-1') }) as Extract<FleetEvent, { kind: 'snapshot' }>,
			T
		);
		expect(f.servedBy).toBe('lab');
		expect(f.cached).toBe(false);
	});

	test('marks when the screen stopped being news, and which status said so', () => {
		const f = new Fleet();
		f.setLink('live');
		f.setLink('down', 503);
		expect(f.down).toBe(503);
		expect(f.asOf).not.toBeNull();
		f.setLink('live');
		expect(f.down).toBeNull();
		expect(f.asOf).toBeNull();
	});
});
