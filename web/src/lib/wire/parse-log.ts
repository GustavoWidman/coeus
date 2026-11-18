/**
 * A job's log stream and closure diff, checked as `parse.ts` checks the fleet stream. Only the
 * log and diff views read these, so they load with them rather than with the app.
 */
import { job, json, list, none, nullable, num, obj, oneOf, outcome, str, tagged } from './parse.ts';
import type { DiffView, LineKind, LogEvent, LogLine, PackageChangeView } from './types.ts';

const line = obj<LogLine>({
	at: nullable(num),
	kind: oneOf<LineKind>('coeus', 'phase', 'fetch', 'build', 'output', 'error', 'done', 'note'),
	text: str,
	prefix: nullable(str)
});

const event = tagged<LogEvent>({
	started: obj({ job }),
	lines: obj({ lines: list(line), end: num }),
	ended: obj({ outcome }),
	expired: none
});

const diff = obj<DiffView>({
	changes: list(
		obj<PackageChangeView>({ name: str, before: list(str), after: list(str), size_delta: num })
	)
});

export const parseLogEvent = (data: string): LogEvent => event(json(data), 'event');

export const parseDiff = (data: string): DiffView => diff(json(data), 'diff');
