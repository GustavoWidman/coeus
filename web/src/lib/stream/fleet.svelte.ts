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
	Viewer
} from '#lib/wire/types.ts';

/** how the stream stands: not answered yet, open, or waiting to try again after it failed or ended */
export type Link = 'connecting' | 'live' | 'retrying';

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
	viewer = $state.raw<Viewer | null>(null);
	link = $state<Link>('connecting');
	/** counts changes to a machine or a deployment; the live dot rings on each */
	beat = $state(0);
	landed = $state.raw<Landed[]>([]);

	get ready() {
		return this.servedBy !== null;
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
