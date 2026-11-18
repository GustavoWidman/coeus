/** Fleet entities for tests, each a sensible default changed by what a test passes. */
import type {
	DeploymentRow,
	DeploymentView,
	Hostname,
	JobCard,
	LiveProgress,
	MachineCard,
	MachineState,
	OrderId
} from '#lib/wire/types.ts';

export const T = 1_794_927_600_000;
export const host = (name: string) => name as Hostname;

export const job = (over: Partial<JobCard> = {}): JobCard => ({
	id: 61,
	order: '2d2eef4459f7ebbf' as OrderId,
	source: { kind: 'git', rev: '2cbe6da7e56b76230b636eccac43a797985e81e1', git_ref: 'main' },
	action: 'switch',
	origin: {
		kind: 'github',
		repository: 'o/nix',
		workflow: 'deploy',
		run_id: 1184,
		attempt: 1,
		url: 'https://github.com/o/nix/actions/runs/1184'
	},
	state: { kind: 'running', phase: 'building' },
	ordered_at: T - 300_000,
	started_at: T - 290_000,
	finished_at: null,
	phase_started_at: T - 200_000,
	toplevel: null,
	attempt: 1,
	changes: null,
	...over
});

export const finished = (
	id: number,
	outcome: Extract<JobCard['state'], { kind: 'finished' }>['outcome'],
	finishedAt: number,
	over: Partial<JobCard> = {}
) =>
	job({
		id,
		state: { kind: 'finished', outcome },
		started_at: finishedAt - 60_000,
		finished_at: finishedAt,
		phase_started_at: null,
		...over
	});

export const state = (over: Partial<MachineState> = {}): MachineState => ({
	seen_at: T,
	platform: 'nixos',
	system: 'x86_64-linux',
	version: '0.4.1',
	daemon: 'serving',
	generation: 1371,
	current: '/nix/store/6rgzl3g7nq77wi3xkrs71xq941vcz1w6-nixos-system-lab-25.11',
	pending_boot: false,
	reboot_required: false,
	deployed: { kind: 'git', rev: '91f0e2a6238c2f455a5265a1111c070585c7553c', git_ref: 'main' },
	follow: { git_ref: 'deployed', latest: null, paused: false },
	running: null,
	next: null,
	recent: [finished(60, { kind: 'succeeded' }, T - 2 * 3_600_000)],
	...over
});

export const card = (
	name: string,
	over: Partial<MachineState> = {},
	link: MachineCard['link'] = { kind: 'connected' }
): MachineCard => ({ host: host(name), link, state: state(over) });

export const progress = (over: Partial<LiveProgress> = {}): LiveProgress => ({
	host: host('lab'),
	job: 61,
	builds: { done: 194, expected: 388, running: 4, failed: 0 },
	downloads: { done: 765, expected: 1530, running: 3, failed: 0 },
	downloaded_bytes: 300 * 1_048_576,
	expected_bytes: 600 * 1_048_576,
	active: ['rustc-1.84.0', 'linux-6.12.9'],
	...over
});

export const row = (name: string, over: Partial<DeploymentRow> = {}): DeploymentRow => ({
	host: host(name),
	wave: 0,
	verdict: 'done',
	job: 1,
	diff: null,
	...over
});

export const deployment = (over: Partial<DeploymentView> = {}): DeploymentView => ({
	order: '2d2eef4459f7ebbf' as OrderId,
	source: { kind: 'git', rev: '2cbe6da7e56b76230b636eccac43a797985e81e1', git_ref: 'main' },
	origin: { kind: 'follow' },
	action: 'switch',
	subject: null,
	ordered_at: T - 400_000,
	settled_at: null,
	waves: [{ name: null, hosts: [host('oracle-1')], started_at: T - 400_000, settled_at: null }],
	rows: [row('oracle-1')],
	...over
});
