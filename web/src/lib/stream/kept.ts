/**
 * The last fleet the stream sent, kept in IndexedDB (`coeus-state`) so a launch shows it at
 * once, marked "connecting", instead of a skeleton, and so a launch while every host is down
 * still shows the fleet as it last was. It lives outside Cache Storage, which every service
 * worker build sweeps, and outlives builds.
 */
import { store } from '#lib/idb.ts';
import { parseFleetEvent } from '#lib/wire/parse.ts';
import type { FleetEvent, Ms } from '#lib/wire/types.ts';

const db = store('coeus-state');
const KEY = 'fleet';
/** at most one write in this long, besides the one as the page hides */
export const EVERY = 30_000;
/** the first write waits this long after the stream first answers, off the first paint */
export const FIRST = 500;

type Snapshot = Extract<FleetEvent, { kind: 'snapshot' }>;
interface Kept {
	/** the snapshot as JSON, read back through the stream's own parser */
	json: string;
	savedAt: Ms;
}

/** the fleet kept last time, and when; nothing where IndexedDB is missing or empty */
export async function load(): Promise<{ snapshot: Snapshot; savedAt: Ms } | null> {
	try {
		const kept = await db.get<Kept>(KEY);
		if (!kept) return null;
		const event = parseFleetEvent(kept.json);
		return event.kind === 'snapshot' ? { snapshot: event, savedAt: kept.savedAt } : null;
	} catch {
		return null;
	}
}

async function save(snapshot: Snapshot) {
	try {
		await db.put(KEY, { json: JSON.stringify(snapshot), savedAt: Date.now() } satisfies Kept);
	} catch {}
}

/**
 * Keeps what `current` returns: soon after the first change, then at most every [EVERY] ms
 * while changes come, and whenever the page hides.
 */
export function keep(current: () => Snapshot | null) {
	let last = 0;
	let timer: ReturnType<typeof setTimeout> | null = null;
	let dirty = false;

	const flush = () => {
		timer = null;
		if (!dirty) return;
		const snapshot = current();
		if (!snapshot) return;
		dirty = false;
		last = Date.now();
		void save(snapshot);
	};

	document.addEventListener('visibilitychange', () => {
		if (document.hidden) flush();
	});

	/** says the fleet changed */
	return () => {
		dirty = true;
		if (timer) return;
		timer = setTimeout(flush, last ? Math.max(0, last + EVERY - Date.now()) : FIRST);
	};
}
