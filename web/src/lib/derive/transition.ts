import type { MachineCard } from '#lib/wire/types.ts';

/** jobs that reached the machine's history since `prev`, newest first */
export function landed(prev: MachineCard | undefined, next: MachineCard): number[] {
	if (!prev?.state || !next.state) return [];
	const had = new Set(prev.state.recent.map((j) => j.id));
	return next.state.recent.filter((j) => !had.has(j.id)).map((j) => j.id);
}
