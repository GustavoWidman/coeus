/**
 * A `coeus web --fixture instrument` of each worker's own, over the built app, its clock held
 * until a test moves it, and pages whose clock agrees with the fleet's.
 */
import { spawn, type ChildProcess } from 'node:child_process';
import { createServer } from 'node:net';
import { resolve } from 'node:path';
import { test as base, type Page } from '@playwright/test';

const WEB = resolve(import.meta.dirname, '../..');
const COEUS = process.env.COEUS_BIN ?? resolve(WEB, '../target/release/coeus');

function freePort(): Promise<number> {
	return new Promise((ok) => {
		const s = createServer();
		s.listen(0, '127.0.0.1', () => {
			const port = (s.address() as { port: number }).port;
			s.close(() => ok(port));
		});
	});
}

export class Fixture {
	constructor(readonly base: string) {}

	/** moves the fleet's clock to `t` seconds into its story and runs it at `speed` (0 holds it) */
	async seek(t: number, speed = 0): Promise<number> {
		const res = await fetch(`${this.base}/fixture/seek?t=${t}&speed=${speed}`, { method: 'POST' });
		if (!res.ok) throw new Error(`seek: ${res.status} ${await res.text()}`);
		return ((await res.json()) as { now: number }).now;
	}
}

type Fixtures = {
	/** opens `path` with the fleet at `t`, its clock running at `speed` */
	open(path: string, t: number, speed?: number): Promise<Page>;
	fleet: Fixture;
};

export const test = base.extend<Fixtures, { server: Fixture }>({
	server: [
		async ({}, use) => {
			const port = await freePort();
			const args = ['web', '--fixture', 'instrument', '--open', '--speed', '0'];
			const proc: ChildProcess = spawn(
				COEUS,
				[...args, '--web-dir', resolve(WEB, 'build'), '--listen', `127.0.0.1:${port}`],
				{ stdio: ['ignore', 'ignore', 'inherit'] }
			);
			const fixture = new Fixture(`http://127.0.0.1:${port}`);
			for (let i = 0; ; i++) {
				try {
					await fixture.seek(0);
					break;
				} catch (e) {
					if (i > 200 || proc.exitCode !== null) throw new Error(`${COEUS} did not start: ${e}`);
					await new Promise((ok) => setTimeout(ok, 50));
				}
			}
			await use(fixture);
			proc.kill();
		},
		{ scope: 'worker' }
	],
	fleet: async ({ server }, use) => use(server),
	open: async ({ page, server }, use) => {
		await use(async (path, t, speed = 0) => {
			const now = await server.seek(t, speed);
			// the page's clock starts where the fleet's is, and runs as the fleet's does
			await page.addInitScript(
				({ now, speed }) => {
					const Real = Date;
					const start = Real.now();
					const at = () => now + (Real.now() - start) * speed;
					const Shifted = function (this: unknown, ...a: unknown[]) {
						if (!new.target) return new Real(at()).toString();
						return a.length ? new (Real as any)(...a) : new Real(at());
					} as unknown as DateConstructor;
					Shifted.now = at;
					Shifted.parse = Real.parse;
					Shifted.UTC = Real.UTC;
					Object.defineProperty(Shifted, 'prototype', { value: Real.prototype });
					globalThis.Date = Shifted;
				},
				{ now, speed }
			);
			await page.goto(server.base + path);
			return page;
		});
	}
});

export { expect } from '@playwright/test';

/** the page as a Home Screen app on an iPhone reports itself */
export const standalone = (page: Page) =>
	page.addInitScript(() =>
		Object.defineProperty(Navigator.prototype, 'standalone', { get: () => true })
	);
