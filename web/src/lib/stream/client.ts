/**
 * Server-sent events read through `fetch` rather than `EventSource`, so the request carries
 * headers like any other: the session, its proof, and `Last-Event-ID`. The stream closes while
 * the page is hidden, where iOS would suspend it anyway.
 */
import { ParseError, parseFleetEvent } from '#lib/wire/parse.ts';
import type { Link } from './fleet.svelte.ts';
import type { FleetEvent } from '#lib/wire/types.ts';

export interface Message {
	id: string | null;
	data: string;
}

/** the complete messages at the start of `text`, and what is left of it */
export function split(text: string): { messages: Message[]; rest: string } {
	const parts = text.replace(/\r\n?/g, '\n').split('\n\n');
	const rest = parts.pop()!;
	const messages: Message[] = [];
	for (const part of parts) {
		let id: string | null = null;
		const data: string[] = [];
		for (const line of part.split('\n')) {
			const colon = line.indexOf(':');
			if (colon === 0) continue;
			const field = colon < 0 ? line : line.slice(0, colon);
			const value = colon < 0 ? '' : line.slice(colon + 1).replace(/^ /, '');
			if (field === 'id') id = value;
			else if (field === 'data') data.push(value);
		}
		if (data.length) messages.push({ id, data: data.join('\n') });
	}
	return { messages, rest };
}

const DELAYS = [500, 1000, 2000, 5000, 10000];
/** what an answer to the stream request means */
type Answer = 'open' | 'refused' | 'failed';

const answer = (status: number): Answer =>
	status >= 200 && status < 300 ? 'open' : status === 401 ? 'refused' : 'failed';

export interface Listener {
	events(events: FleetEvent[]): void;
	link(state: Link): void;
	/** the headers that prove this request's session */
	sign(path: string): Promise<Record<string, string>>;
	/** the session was refused or has ended; resolves once there is another */
	refused(): Promise<void>;
}

export function connect(url: string, on: Listener) {
	let cursor: string | null = null;
	let failures = 0;
	let abort: AbortController | null = null;
	let wake: (() => void) | null = null;
	const path = new URL(url, location.href);

	const visible = () =>
		new Promise<void>((ok) => {
			if (!document.hidden) return ok();
			wake = ok;
		});

	document.addEventListener('visibilitychange', () => {
		if (document.hidden) abort?.abort();
		else wake?.();
	});

	/** reads messages until the stream ends; true if it ended because the session did */
	async function read(body: ReadableStream<Uint8Array>) {
		const reader = body.getReader();
		const text = new TextDecoder();
		let rest = '';
		for (;;) {
			const { done, value } = await reader.read();
			if (done) return false;
			const got = split(rest + text.decode(value, { stream: true }));
			rest = got.rest;
			const events: FleetEvent[] = [];
			let expired = false;
			for (const m of got.messages) {
				try {
					const e = parseFleetEvent(m.data);
					if (e.kind === 'expired') expired = true;
					else events.push(e);
				} catch (e) {
					if (!(e instanceof ParseError)) throw e;
					console.warn(`coeus: dropped a stream message: ${e.message}`);
				}
				if (m.id !== null) cursor = m.id;
			}
			if (events.length) on.events(events);
			if (expired) return true;
		}
	}

	async function run() {
		for (;;) {
			await visible();
			abort = new AbortController();
			let got: Answer = 'failed';
			try {
				const headers = await on.sign(path.pathname + path.search);
				if (cursor) headers['last-event-id'] = cursor;
				const res = await fetch(url, { headers, cache: 'no-store', signal: abort.signal });
				got = answer(res.status);
				if (got === 'open' && res.body) {
					// a stream resumed with nothing new says nothing until something changes
					failures = 0;
					on.link('live');
					if (await read(res.body)) got = 'refused';
				} else await res.body?.cancel();
			} catch {}
			if (got === 'refused') {
				on.link('connecting');
				await on.refused();
				failures = 0;
				continue;
			}
			on.link('retrying');
			if (document.hidden) continue;
			const delay = DELAYS[Math.min(failures++, DELAYS.length - 1)];
			await new Promise((ok) => setTimeout(ok, delay * (0.75 + Math.random() * 0.5)));
		}
	}

	run();
}
