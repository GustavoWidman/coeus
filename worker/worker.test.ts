// Runs against a built module: OFFLINE_WORKER is its path and OFFLINE_WORKER_HOSTS the hosts it
// was built for, as `nix flake check` passes them.
import { afterEach, describe, expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

const path = process.env.OFFLINE_WORKER;
const built = process.env.OFFLINE_WORKER_HOSTS?.split(' ');
if (!path || !built) throw new Error('OFFLINE_WORKER and OFFLINE_WORKER_HOSTS must be set');
const source = readFileSync(path, 'utf8');
const worker: { fetch(request: Request): Promise<Response> } = (await import(path)).default;

const realFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = realFetch;
});

/** Points the Worker's fetch at an origin, and records what reached it. */
function origin(answer: (request: Request) => Response | Promise<Response>) {
	const seen: Request[] = [];
	globalThis.fetch = (async (input: Request) => {
		seen.push(input);
		return answer(input);
	}) as typeof fetch;
	return seen;
}

const url = 'https://coeus.example.com/fleet';
const navigate = (init: RequestInit = {}) =>
	new Request(url, {
		...init,
		headers: { 'sec-fetch-mode': 'navigate', accept: 'text/html', ...init.headers }
	});
const script = (init: RequestInit = {}) =>
	new Request(url, { ...init, headers: { 'sec-fetch-mode': 'cors', ...init.headers } });

const tunnelDown = () => new Response('error code: 1033', { status: 530 });

function expectOffline(res: Response) {
	expect(res.status).toBe(503);
	expect(res.headers.get('cache-control')).toBe('no-store');
	expect(res.headers.get('retry-after')).toBe('15');
	expect(res.headers.get('coeus-offline')).toBe('1');
}

async function expectPage(res: Response) {
	expectOffline(res);
	expect(res.headers.get('content-type')).toBe('text/html; charset=utf-8');
	expect(res.headers.get('content-security-policy')).toStartWith("default-src 'none'");
	const html = await res.text();
	expect(html).toContain('<h1>No coeus host answered</h1>');
	return html;
}

async function expectJson(res: Response) {
	expectOffline(res);
	expect(res.headers.get('content-type')).toBe('application/json');
	expect(res.headers.has('content-security-policy')).toBe(false);
	expect(await res.json()).toEqual({ offline: true });
}

describe('an origin that answers', () => {
	test('passes the very response through, body unread', async () => {
		const answer = new Response('<!doctype html>', {
			status: 200,
			headers: { 'content-type': 'text/html', etag: '"abc"' }
		});
		origin(() => answer);
		const res = await worker.fetch(navigate());
		expect(res).toBe(answer);
		expect(res.bodyUsed).toBe(false);
	});

	test('passes a stream through as it arrives', async () => {
		let push!: (chunk: string) => void;
		const body = new ReadableStream<Uint8Array>({
			start(c) {
				push = (chunk) => c.enqueue(new TextEncoder().encode(chunk));
			}
		});
		origin(() => new Response(body, { headers: { 'content-type': 'text/event-stream' } }));
		const res = await worker.fetch(script());
		const reader = res.body!.getReader();
		push('event: snapshot\n\n');
		expect(new TextDecoder().decode((await reader.read()).value)).toBe('event: snapshot\n\n');
	});

	test.each([500, 502, 503, 504, 520, 521, 522, 523, 524])(
		'passes a %i through unchanged',
		async (status) => {
			const answer = new Response('origin says no', { status });
			origin(() => answer);
			expect(await worker.fetch(navigate())).toBe(answer);
			expect(await worker.fetch(script())).toBe(answer);
		}
	);

	test('hands the origin the request as it came, method and body included', async () => {
		const seen = origin(() => new Response(null, { status: 204 }));
		const request = script({ method: 'POST', body: '{"a":1}' });
		const res = await worker.fetch(request);
		expect(res.status).toBe(204);
		expect(seen).toEqual([request]);
		expect(seen[0].method).toBe('POST');
	});

	test('passes HEAD through', async () => {
		const answer = new Response(null, { status: 200, headers: { 'content-length': '1234' } });
		const seen = origin(() => answer);
		expect(await worker.fetch(navigate({ method: 'HEAD' }))).toBe(answer);
		expect(seen[0].method).toBe('HEAD');
	});
});

describe('no connector (530)', () => {
	test('a navigation gets the offline page', async () => {
		origin(tunnelDown);
		await expectPage(await worker.fetch(navigate()));
	});

	test('a fetch from a script gets JSON', async () => {
		origin(tunnelDown);
		await expectJson(await worker.fetch(script()));
	});

	test('a script that asks for HTML still gets JSON', async () => {
		origin(tunnelDown);
		await expectJson(await worker.fetch(script({ headers: { accept: 'text/html' } })));
	});

	test('without Sec-Fetch-Mode, a request that accepts HTML gets the page', async () => {
		origin(tunnelDown);
		await expectPage(await worker.fetch(new Request(url, { headers: { accept: 'text/html' } })));
		await expectJson(await worker.fetch(new Request(url, { headers: { accept: '*/*' } })));
	});

	test('a HEAD navigation gets the page headers and no body', async () => {
		origin(tunnelDown);
		const res = await worker.fetch(navigate({ method: 'HEAD' }));
		expectOffline(res);
		expect(res.headers.get('content-type')).toBe('text/html; charset=utf-8');
		expect(res.body).toBeNull();
	});

	test('a HEAD from a script, as the page checks with, gets the JSON headers and no body', async () => {
		origin(tunnelDown);
		const res = await worker.fetch(script({ method: 'HEAD' }));
		expectOffline(res);
		expect(res.headers.get('content-type')).toBe('application/json');
		expect(res.body).toBeNull();
	});

	test.each(['POST', 'PUT', 'DELETE', 'PATCH'])(
		'a %s gets JSON, even as a navigation',
		async (method) => {
			origin(tunnelDown);
			await expectJson(await worker.fetch(navigate({ method, body: 'x' })));
			await expectJson(await worker.fetch(script({ method, body: 'x' })));
		}
	);
});

describe('an origin that cannot be reached', () => {
	const unreachable = () => {
		throw new TypeError('Network connection lost.');
	};

	test('a navigation gets the offline page', async () => {
		origin(unreachable);
		await expectPage(await worker.fetch(navigate()));
	});

	test('a fetch from a script gets JSON', async () => {
		origin(unreachable);
		await expectJson(await worker.fetch(script()));
	});

	test('a rejected promise counts as unreachable too', async () => {
		origin(() => Promise.reject(new TypeError('fetch failed')));
		await expectJson(await worker.fetch(script({ method: 'POST', body: 'x' })));
	});
});

describe('the page', () => {
	const page = async () => {
		origin(tunnelDown);
		const res = await worker.fetch(navigate());
		return { csp: res.headers.get('content-security-policy')!, html: await res.text() };
	};
	const sha256 = (text: string) => `'sha256-${createHash('sha256').update(text).digest('base64')}'`;
	const inline = (html: string, tag: string) =>
		[...html.matchAll(new RegExp(`<${tag}>([\\s\\S]*?)</${tag}>`, 'g'))].map((m) => m[1]);

	test('its policy names the hash of each inline style and script, and nothing else runs', async () => {
		const { csp, html } = await page();
		const directives = new Map(
			csp.split(';').map((d) => {
				const [name, ...values] = d.trim().split(/\s+/);
				return [name, values];
			})
		);
		expect(directives.get('default-src')).toEqual(["'none'"]);
		expect(directives.get('style-src')).toEqual(inline(html, 'style').map(sha256));
		expect(directives.get('script-src')).toEqual(inline(html, 'script').map(sha256));
		expect(inline(html, 'style')).toHaveLength(1);
		expect(inline(html, 'script')).toHaveLength(1);
		expect(directives.get('frame-ancestors')).toEqual(["'none'"]);
		// what a hash cannot allow: attributes that style or run something
		expect(html).not.toMatch(/\sstyle=/);
		expect(html).not.toMatch(/\son[a-z]+=/);
		expect(html).not.toMatch(/<script\s[^>]*src=/);
	});

	test('it makes no request but its own check', async () => {
		const { html } = await page();
		expect(html).not.toMatch(/url\(|@import|<link rel="(?:stylesheet|preload|manifest)/);
		const links = [...html.matchAll(/\s(?:href|src)="([^"]*)"/g)].map((m) => m[1]);
		expect(links.every((l) => l.startsWith('data:'))).toBe(true);
	});

	test('it carries the theme for light and dark', async () => {
		const { html } = await page();
		expect(html).toContain('--bg:');
		expect(html).toContain('@media (prefers-color-scheme: dark)');
	});

	test('it lists the hosts it was built for', async () => {
		const { html } = await page();
		const hosts = [...html.matchAll(/<li><i class="dot"><\/i><span>([^<]+)<\/span>/g)].map(
			(m) => m[1]
		);
		expect(hosts).toEqual(built);
		expect(html).toContain(`<span class="v">${built.length}</span>`);
	});

	test('the module has no placeholder left', () => {
		expect(source).not.toMatch(/__PAGE__|__CSP__|@(?:style|script|icon|count|hosts)@/);
	});
});
