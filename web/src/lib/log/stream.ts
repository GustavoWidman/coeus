/**
 * One job's log, followed as the fleet stream is: server-sent events read through `fetch`,
 * closed while the page is hidden. Each message's id is the machine's log offset, which a
 * reconnect sends back so the log goes on where it stopped, whichever host answers.
 */
import { split } from '#lib/stream/client.ts';
import { ParseError } from '#lib/wire/parse.ts';
import { parseLogEvent } from '#lib/wire/parse-log.ts';
import type { JobCard, LogLine, OutcomeView } from '#lib/wire/types.ts';

export interface LogListener {
	started(job: JobCard): void;
	lines(lines: LogLine[]): void;
	ended(outcome: OutcomeView): void;
	/** why the log cannot be read right now, or null once it can again */
	failed(why: string | null): void;
	/** the headers that prove this request's session */
	sign(path: string): Promise<Record<string, string>>;
}

const DELAYS = [500, 1000, 2000, 5000, 10000];

/** follows the log at `url` until the job ends; the answer stops following it */
export function follow(url: string, on: LogListener): () => void {
	const path = new URL(url, location.href);
	let offset: string | null = null;
	let stopped = false;
	let failures = 0;
	let abort: AbortController | null = null;
	let wake: (() => void) | null = null;

	const visible = () =>
		new Promise<void>((ok) => {
			if (!document.hidden) return ok();
			wake = ok;
		});
	const shown = () => {
		if (document.hidden) abort?.abort();
		else wake?.();
	};
	document.addEventListener('visibilitychange', shown);

	/** reads the stream, and answers whether the log is over */
	async function read(body: ReadableStream<Uint8Array>): Promise<boolean> {
		const reader = body.getReader();
		const text = new TextDecoder();
		let rest = '';
		for (;;) {
			const { done, value } = await reader.read();
			if (done) return false;
			const got = split(rest + text.decode(value, { stream: true }));
			rest = got.rest;
			for (const m of got.messages) {
				if (m.id !== null) offset = m.id;
				let e;
				try {
					e = parseLogEvent(m.data);
				} catch (err) {
					if (!(err instanceof ParseError)) throw err;
					console.warn(`coeus: dropped a log message: ${err.message}`);
					continue;
				}
				if (e.kind === 'started') on.started(e.job);
				else if (e.kind === 'lines') on.lines(e.lines);
				else if (e.kind === 'ended') on.ended(e.outcome);
				if (e.kind === 'ended' || e.kind === 'expired') return true;
			}
		}
	}

	async function run() {
		while (!stopped) {
			await visible();
			if (stopped) return;
			abort = new AbortController();
			try {
				const headers = await on.sign(path.pathname + path.search);
				if (offset) headers['last-event-id'] = offset;
				const res = await fetch(url, { headers, cache: 'no-store', signal: abort.signal });
				if (res.ok && res.body) {
					failures = 0;
					on.failed(null);
					if (await read(res.body)) return;
				} else {
					const why = (await res.text()).split('\n')[0];
					on.failed(why || `the server answered ${res.status}`);
				}
			} catch (e) {
				if (stopped) return;
				if (!document.hidden) on.failed(e instanceof Error ? e.message : String(e));
			}
			if (stopped || document.hidden) continue;
			const delay = DELAYS[Math.min(failures++, DELAYS.length - 1)];
			await new Promise((ok) => setTimeout(ok, delay * (0.75 + Math.random() * 0.5)));
		}
	}

	run();
	return () => {
		stopped = true;
		abort?.abort();
		wake?.();
		document.removeEventListener('visibilitychange', shown);
	};
}
