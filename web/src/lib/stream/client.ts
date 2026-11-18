/**
 * Server-sent events read through `fetch` rather than `EventSource`, so the request carries
 * headers like any other, `Last-Event-ID` among them. The stream closes while the page is
 * hidden, where iOS would suspend it anyway.
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

export interface Listener {
	events(events: FleetEvent[]): void;
	link(state: Link): void;
}

export function connect(url: string, on: Listener) {
	let cursor: string | null = null;
	let failures = 0;
	let abort: AbortController | null = null;
	let wake: (() => void) | null = null;

	const visible = () =>
		new Promise<void>((ok) => {
			if (!document.hidden) return ok();
			wake = ok;
		});

	document.addEventListener('visibilitychange', () => {
		if (document.hidden) abort?.abort();
		else wake?.();
	});

	async function read(body: ReadableStream<Uint8Array>) {
		const reader = body.getReader();
		const text = new TextDecoder();
		let rest = '';
		for (;;) {
			const { done, value } = await reader.read();
			if (done) return;
			const got = split(rest + text.decode(value, { stream: true }));
			rest = got.rest;
			const events: FleetEvent[] = [];
			for (const m of got.messages) {
				try {
					events.push(parseFleetEvent(m.data));
				} catch (e) {
					if (!(e instanceof ParseError)) throw e;
					console.warn(`coeus: dropped a stream message: ${e.message}`);
				}
				if (m.id !== null) cursor = m.id;
			}
			if (events.length) on.events(events);
		}
	}

	async function run() {
		for (;;) {
			await visible();
			abort = new AbortController();
			try {
				const res = await fetch(url, {
					headers: cursor ? { 'Last-Event-ID': cursor } : {},
					cache: 'no-store',
					signal: abort.signal
				});
				if (res.ok && res.body) {
					// a stream resumed with nothing new says nothing until something changes
					failures = 0;
					on.link('live');
					await read(res.body);
				}
			} catch {}
			on.link('retrying');
			if (document.hidden) continue;
			const delay = DELAYS[Math.min(failures++, DELAYS.length - 1)];
			await new Promise((ok) => setTimeout(ok, delay * (0.75 + Math.random() * 0.5)));
		}
	}

	run();
}
