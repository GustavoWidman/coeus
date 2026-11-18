/**
 * The stream's messages, checked against the types the server declares. Everything past here
 * trusts its types. A field the app does not know is dropped, so a newer server may add fields.
 */
import type {
	DeploymentRow,
	DeploymentView,
	FleetEvent,
	FollowView,
	Hostname,
	JobCard,
	JobStateView,
	LinkView,
	LiveProgress,
	MachineCard,
	MachineState,
	OrderId,
	OriginView,
	OutcomeView,
	PhaseView,
	SourceView,
	TallyView,
	UnreachView,
	Viewer,
	WaveView
} from './types.ts';

export class ParseError extends Error {}

type Parser<T> = (value: unknown, at: string) => T;

const fail = (at: string, want: string): never => {
	throw new ParseError(`${at}: not ${want}`);
};

const str: Parser<string> = (v, at) => (typeof v === 'string' ? v : fail(at, 'a string'));
const num: Parser<number> = (v, at) =>
	typeof v === 'number' && Number.isFinite(v) ? v : fail(at, 'a number');
const bool: Parser<boolean> = (v, at) => (typeof v === 'boolean' ? v : fail(at, 'true or false'));
const host = str as Parser<Hostname>;
const order = str as Parser<OrderId>;

const nullable =
	<T>(p: Parser<T>): Parser<T | null> =>
	(v, at) =>
		v === null ? null : p(v, at);

const list =
	<T>(p: Parser<T>): Parser<T[]> =>
	(v, at) =>
		Array.isArray(v) ? v.map((x, i) => p(x, `${at}[${i}]`)) : fail(at, 'a list');

const oneOf =
	<T extends string>(...values: T[]): Parser<T> =>
	(v, at) =>
		values.includes(v as T) ? (v as T) : fail(at, values.join(' or '));

const record = (v: unknown, at: string) =>
	typeof v === 'object' && v !== null && !Array.isArray(v)
		? (v as Record<string, unknown>)
		: fail(at, 'an object');

function obj<T>(shape: { [K in keyof T]-?: Parser<T[K]> }): Parser<T> {
	return (v, at) => {
		const r = record(v, at);
		const out = {} as T;
		for (const k in shape) out[k] = shape[k](r[k], `${at}.${k}`);
		return out;
	};
}

type Cases<T extends { kind: string }> = {
	[K in T['kind']]: Parser<Omit<Extract<T, { kind: K }>, 'kind'>>;
};

function tagged<T extends { kind: string }>(cases: Cases<T>): Parser<T> {
	return (v, at) => {
		const kind = record(v, at).kind as T['kind'];
		if (typeof kind !== 'string' || !Object.hasOwn(cases, kind))
			fail(`${at}.kind`, Object.keys(cases).join(' or '));
		return { kind, ...cases[kind](v, at) } as unknown as T;
	};
}

const none: Parser<Record<never, never>> = (v, at) => (record(v, at), {});

const phase: Parser<PhaseView> = oneOf(
	'preparing',
	'fetching',
	'evaluating',
	'building',
	'activating',
	'verifying'
);

const source = tagged<SourceView>({
	git: obj({ rev: str, git_ref: nullable(str) }),
	snapshot: obj({ label: str, nar_hash: str }),
	generation: obj({ number: num })
});

const origin = tagged<OriginView>({
	user: obj({ user: str, host: str }),
	follow: none,
	github: obj({
		repository: str,
		workflow: str,
		run_id: num,
		attempt: num,
		url: str
	})
});

const failure = obj({ phase, message: str });
const outcome = tagged<OutcomeView>({
	succeeded: none,
	up_to_date: none,
	cancelled: none,
	interrupted: none,
	superseded: obj({ by: num }),
	failed: failure,
	rolled_back: failure
});

const job = obj<JobCard>({
	id: num,
	order,
	source,
	action: oneOf('switch', 'boot', 'test', 'build'),
	origin,
	state: tagged<JobStateView>({
		queued: none,
		running: obj({ phase }),
		finished: obj({ outcome })
	}),
	ordered_at: num,
	started_at: nullable(num),
	finished_at: nullable(num),
	phase_started_at: nullable(num),
	toplevel: nullable(str),
	attempt: num,
	changes: nullable(num)
});

const link = tagged<LinkView>({
	connected: none,
	connecting: none,
	unreachable: obj({
		since: num,
		reason: tagged<UnreachView>({
			offline: obj({ last_seen: nullable(num) }),
			refused: obj({ detail: str }),
			failed: obj({ detail: str })
		})
	})
});

const follow = obj<FollowView>({
	git_ref: str,
	latest: nullable(obj({ rev: str, committed_at: num })),
	paused: bool
});

const state = obj<MachineState>({
	seen_at: num,
	platform: oneOf('nixos', 'darwin'),
	system: str,
	version: str,
	daemon: oneOf('serving', 'restarting'),
	generation: nullable(num),
	current: nullable(str),
	pending_boot: bool,
	reboot_required: bool,
	deployed: nullable(source),
	follow: nullable(follow),
	running: nullable(job),
	next: nullable(job),
	recent: list(job)
});

const machine = obj<MachineCard>({ host, link, state: nullable(state) });

const tally = obj<TallyView>({ done: num, expected: num, running: num, failed: num });

const progress = obj<LiveProgress>({
	host,
	job: num,
	builds: tally,
	downloads: tally,
	downloaded_bytes: num,
	expected_bytes: num,
	active: list(str)
});

const wave = obj<WaveView>({
	name: nullable(str),
	hosts: list(host),
	started_at: num,
	settled_at: nullable(num)
});

const row = obj<DeploymentRow>({
	host,
	wave: nullable(num),
	verdict: oneOf('waiting', 'running', 'done', 'failed', 'pending', 'catching_up', 'caught_up'),
	job: nullable(num),
	diff: nullable(obj({ changes: num, size_delta: num }))
});

const deployment = obj<DeploymentView>({
	order,
	source,
	origin,
	action: oneOf('switch', 'boot', 'test', 'build'),
	subject: nullable(str),
	ordered_at: num,
	settled_at: nullable(num),
	waves: list(wave),
	rows: list(row)
});

const viewer = obj<Viewer>({ login: str, device: str, expires_at: num });

const event = tagged<FleetEvent>({
	snapshot: obj({
		schema: num,
		build: str,
		served_by: host,
		viewer,
		machines: list(machine),
		progress: list(progress),
		deployments: list(deployment)
	}),
	machine: obj({ machine }),
	progress: obj({ progress }),
	progress_cleared: obj({ host }),
	deployment: obj({ deployment }),
	deployment_removed: obj({ order }),
	expired: none
});

export function parseFleetEvent(data: string): FleetEvent {
	let json: unknown;
	try {
		json = JSON.parse(data);
	} catch {
		throw new ParseError('not JSON');
	}
	return event(json, 'event');
}
