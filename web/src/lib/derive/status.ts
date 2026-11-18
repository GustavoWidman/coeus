import type {
	DeploymentRow,
	DeploymentView,
	JobCard,
	LiveProgress,
	MachineCard,
	Ms,
	OutcomeView,
	PhaseView,
	TallyView,
	Verdict
} from '#lib/wire/types.ts';
import { ago, source } from './format.ts';

export const PHASES: PhaseView[] = [
	'preparing',
	'fetching',
	'evaluating',
	'building',
	'activating',
	'verifying'
];
/** each phase's share of a job's progress line */
const WEIGHTS = [0.02, 0.05, 0.13, 0.66, 0.09, 0.05];
const BUILDING = 3;

export type Dot = 'run' | 'ok' | 'bad' | 'warn' | 'idle' | 'wait' | 'asleep' | 'stale';

export interface Status {
	dot: Dot;
	text: string;
	tone?: 'bad' | 'warn' | 'mute';
	/** builds done of expected, shown after the text while a job builds */
	builds?: TallyView;
	p?: number;
}

export const progressOf = (job: JobCard | null, p: LiveProgress | undefined) =>
	job && p && p.job === job.id ? p : undefined;

const ratio = (t: TallyView) => (t.expected ? Math.min(1, t.done / t.expected) : 0);

/** the phases behind a running job, and within building its builds and downloads */
export function fraction(job: JobCard, p: LiveProgress | undefined) {
	if (job.state.kind !== 'running') return 0;
	const i = PHASES.indexOf(job.state.phase);
	let f = 0;
	for (let k = 0; k < i; k++) f += WEIGHTS[k];
	p = progressOf(job, p);
	if (i === BUILDING && p) f += WEIGHTS[i] * (0.75 * ratio(p.builds) + 0.25 * ratio(p.downloads));
	return f;
}

export function outcome(o: OutcomeView) {
	switch (o.kind) {
		case 'succeeded':
			return 'succeeded';
		case 'up_to_date':
			return 'up to date';
		case 'cancelled':
			return 'cancelled';
		case 'interrupted':
			return 'interrupted';
		case 'superseded':
			return `superseded by #${o.by}`;
		case 'failed':
			return `failed in ${o.phase}: ${o.message}`;
		case 'rolled_back':
			return `rolled back after ${o.phase}: ${o.message}`;
	}
}

export const OUTCOME_DOT: Record<OutcomeView['kind'], Dot> = {
	succeeded: 'ok',
	up_to_date: 'idle',
	failed: 'bad',
	cancelled: 'idle',
	superseded: 'idle',
	interrupted: 'warn',
	rolled_back: 'warn'
};

export function status(card: MachineCard, p: LiveProgress | undefined, now: Ms): Status {
	const s = card.state;
	if (card.link.kind === 'unreachable') {
		const r = card.link.reason;
		if (r.kind === 'refused') return { dot: 'bad', text: `refused · ${r.detail}`, tone: 'bad' };
		if (r.kind === 'failed')
			return { dot: 'stale', text: `unreachable · ${r.detail}`, tone: 'warn' };
		const seen = r.last_seen ?? s?.seen_at;
		return {
			dot: 'asleep',
			text: `asleep · will catch up${seen ? ` · seen ${ago(seen, now)}` : ''}`,
			tone: 'mute'
		};
	}
	if (!s) return { dot: 'wait', text: 'connecting', tone: 'mute' };

	const job = s.running;
	if (job?.state.kind === 'running') {
		const phase = job.state.phase;
		const text = (job.origin.kind === 'follow' ? 'catching up · ' : '') + phase;
		const pr = progressOf(job, p);
		const builds = phase === 'building' && pr ? pr.builds : undefined;
		return { dot: 'run', text, builds, p: fraction(job, p) };
	}
	if (s.next)
		return {
			dot: 'wait',
			text: `starting #${s.next.id} ${s.next.action} · ${source(s.next.source)}`
		};

	const last = s.recent[0];
	const end = last?.finished_at;
	if (
		last?.state.kind === 'finished' &&
		last.state.outcome.kind === 'failed' &&
		end &&
		now - end < 36e5
	)
		return {
			dot: 'bad',
			text: `failed in ${last.state.outcome.phase} · ${ago(end, now)}`,
			tone: 'bad'
		};
	if (s.follow?.paused)
		return {
			dot: 'warn',
			text: `follow paused${s.deployed ? ` · ${source(s.deployed)}` : ''}`,
			tone: 'warn'
		};
	const tail = s.reboot_required
		? 'reboot required'
		: s.pending_boot
			? 'pending boot'
			: end
				? ago(end, now)
				: '';
	const said =
		last?.state.kind === 'finished' ? outcome(last.state.outcome).split(':')[0] : 'up to date';
	return { dot: 'ok', text: tail ? `${said} · ${tail}` : said };
}

export function verdictDot(v: Verdict, card: MachineCard | undefined): Dot {
	switch (v) {
		case 'done':
		case 'caught_up':
			return 'ok';
		case 'running':
		case 'catching_up':
			return 'run';
		case 'failed':
			return 'bad';
		case 'waiting':
			return 'wait';
		case 'pending':
			return card?.link.kind === 'unreachable' ? 'asleep' : 'wait';
	}
}

export function share(
	row: DeploymentRow,
	card: MachineCard | undefined,
	p: LiveProgress | undefined
) {
	switch (row.verdict) {
		case 'running':
		case 'catching_up': {
			const job = card?.state?.running;
			return job ? fraction(job, p) : 0;
		}
		case 'failed':
			return 0.62;
		case 'done':
		case 'caught_up':
			return 1;
		default:
			return 0;
	}
}

export function counts(d: DeploymentView) {
	const c = { done: 0, run: 0, fail: 0, pend: 0 };
	for (const r of d.rows) {
		if (r.verdict === 'done' || r.verdict === 'caught_up') c.done++;
		else if (r.verdict === 'running' || r.verdict === 'catching_up') c.run++;
		else if (r.verdict === 'failed') c.fail++;
		else c.pend++;
	}
	return c;
}

/** the newest deployment ordered to more than one machine; one ordered to a single machine shows in its history */
export const latest = (deployments: DeploymentView[]) =>
	deployments.find((d) => d.rows.filter((r) => r.wave !== null).length > 1);

/** a deployment's machines as the screens group them: each wave, then those following the ref */
export interface Column {
	name: string;
	rows: DeploymentRow[];
	/** a wave the deploy ordered, as opposed to the machines that catch up on their own */
	ordered: boolean;
	started: Ms | null;
	settled: Ms | null;
	done: boolean;
}

export function columns(d: DeploymentView): Column[] {
	const cols: Column[] = d.waves.map((w, i) => ({
		name: w.name ?? `wave ${i + 1}`,
		rows: d.rows.filter((r) => r.wave === i),
		ordered: true,
		started: w.started_at,
		settled: w.settled_at,
		done: w.settled_at !== null
	}));
	const following = d.rows.filter((r) => r.wave === null);
	if (following.length)
		cols.push({
			name: 'catch-up',
			rows: following,
			ordered: false,
			started: null,
			settled: null,
			done: following.every((r) => r.verdict === 'caught_up')
		});
	return cols;
}
