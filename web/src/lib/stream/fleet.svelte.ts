/**
 * The fleet as the stream last described it. Every entity arrives whole and replaces the one
 * before it, so each lives in a raw value that changes identity only when the entity does.
 */
import { landed } from '#lib/derive/transition.ts';
import type {
	DeploymentView,
	FleetEvent,
	Hostname,
	LiveProgress,
	MachineCard,
	Ms,
	Viewer
} from '#lib/wire/types.ts';

/**
 * how the stream stands: not answered yet, open, waiting to try again after it failed or ended,
 * or waiting because no host serves the site at all
 */
export type Link = 'connecting' | 'live' | 'retrying' | 'down';

/** a job that just reached a machine's history, and when the page saw it arrive */
export interface Landed {
	host: Hostname;
	job: number;
	at: number;
}

const sameList = (a: string[], b: string[]) =>
	a.length === b.length && a.every((x, i) => x === b[i]);

export class Fleet {
	machines = $state.raw<MachineCard[]>([]);
	progress = $state.raw<Partial<Record<Hostname, LiveProgress>>>({});
	deployments = $state.raw<DeploymentView[]>([]);
	servedBy = $state.raw<Hostname | null>(null);
	schema = 0;
	build = '';
	viewer = $state.raw<Viewer | null>(null);
	link = $state<Link>('connecting');
	/** the status that said every host is down, while one did */
	down = $state<number | null>(null);
	/** when what is on screen was last known to be true, once it may no longer be */
	asOf = $state<Ms | null>(null);
	/** what is on screen came from this device's cache, and the stream has not replaced it */
	cached = $state(false);
	/** counts changes to a machine or a deployment; the live dot rings on each */
	beat = $state(0);
	landed = $state.raw<Landed[]>([]);

	get ready() {
		return this.servedBy !== null;
	}

	/** the stream's state; leaving `live` marks the moment what is on screen stopped being news */
	setLink(link: Link, status?: number) {
		if (this.link === 'live' && link !== 'live') this.asOf = Date.now();
		if (link === 'live') this.asOf = null;
		this.down = link === 'down' ? (status ?? 530) : null;
		this.link = link;
	}

	/** shows a snapshot this device kept from `savedAt`, until the stream sends its own */
	restore(snapshot: Extract<FleetEvent, { kind: 'snapshot' }>, savedAt: Ms) {
		if (this.ready) return;
		this.one(snapshot);
		this.cached = true;
		if (this.link !== 'live') this.asOf = savedAt;
	}

	/** the fleet as a snapshot, to keep for the next launch */
	snapshot(): Extract<FleetEvent, { kind: 'snapshot' }> | null {
		if (!this.servedBy || !this.viewer) return null;
		return {
			kind: 'snapshot',
			schema: this.schema,
			build: this.build,
			served_by: this.servedBy,
			viewer: this.viewer,
			machines: this.machines,
			progress: Object.values(this.progress) as LiveProgress[],
			deployments: this.deployments
		};
	}

	machine(host: string) {
		return this.machines.find((m) => m.host === host);
	}

	/** whether the event moves things on screen, rather than changing numbers in place */
	structural(e: FleetEvent) {
		if (e.kind === 'expired') return false;
		if (e.kind !== 'progress') return true;
		const was = this.progress[e.progress.host];
		return !was || was.job !== e.progress.job || !sameList(was.active, e.progress.active);
	}

	apply(events: FleetEvent[]) {
		let changed = false;
		for (const e of events) {
			changed ||= e.kind !== 'progress' && this.structural(e);
			this.one(e);
		}
		if (changed) this.beat++;
	}

	private one(e: FleetEvent) {
		switch (e.kind) {
			case 'snapshot':
				this.cached = false;
				this.schema = e.schema;
				this.build = e.build;
				this.machines = e.machines;
				this.progress = Object.fromEntries(e.progress.map((p) => [p.host, p]));
				this.deployments = e.deployments;
				this.servedBy = e.served_by;
				this.viewer = e.viewer;
				return;
			case 'machine': {
				const next = e.machine;
				const i = this.machines.findIndex((m) => m.host === next.host);
				const jobs = landed(this.machines[i], next);
				this.machines = i < 0 ? [...this.machines, next] : this.machines.with(i, next);
				if (jobs.length) {
					const at = Date.now();
					this.landed = [
						...this.landed.filter((l) => l.host !== next.host),
						...jobs.map((job) => ({ host: next.host, job, at }))
					];
				}
				return;
			}
			case 'progress':
				this.progress = { ...this.progress, [e.progress.host]: e.progress };
				return;
			case 'progress_cleared': {
				const { [e.host]: _, ...rest } = this.progress;
				this.progress = rest;
				return;
			}
			case 'deployment': {
				const d = e.deployment;
				this.deployments = [...this.deployments.filter((x) => x.order !== d.order), d].sort(
					(a, b) => b.ordered_at - a.ordered_at
				);
				return;
			}
			case 'deployment_removed':
				this.deployments = this.deployments.filter((d) => d.order !== e.order);
				return;
			case 'expired':
				return;
		}
	}
}

export const fleet = new Fleet();
