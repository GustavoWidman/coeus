/**
 * The state matrix: every state the dashboard can show (tests/matrix/states.ts) in light and
 * dark, on a phone and a desktop, with full and reduced motion. Each cell opens the built app
 * against `coeus web --fixture instrument` with the fleet's clock where the state needs it,
 * takes a screenshot, and measures what the page costs while it sits still for 2 s, and while
 * 10 s of live updates stream in where the state moves. Once those stop, the clock is held where
 * they left it and the page sits still for 2 s more, so DOM growth is measured in a state that
 * does not change: a leak, not what the updates added.
 *
 *   bun scripts/matrix.ts [--only screen=fleet,log] [--only state=building] [--only target=app]
 *     [--engines chromium,webkit] [--workers 4] [--window 10] [--repeat 1]
 *     [--out test-results/matrix] [--trace] [--reference <prototype dir>] [--web-dir build]
 *     [--coeus <binary>] [--no-timings] [--no-gate] [--update]
 *   bun scripts/matrix.ts --from <matrix-results.json> [--no-timings] [--update]
 *
 * Chromium measures every cell through the DevTools protocol: long tasks, frames presented and
 * dropped, layouts, style recalcs, paints and the area they cover, main-thread time,
 * composited layers, DOM nodes, the JS heap, and GPU memory where Chromium reports it. WebKit,
 * the phone's engine, photographs the phone cells and times their frames. In both, every
 * animation the page starts is recorded with the property it animates.
 *
 * The run writes matrix-results.json and a screenshot per cell into --out, then checks the
 * app's cells against matrix-budget.json and tests/matrix/baselines/<platform>.json (screenshot
 * hashes, where that file exists). Counts are checked everywhere; frame timing and GPU memory
 * only without --no-timings, since a machine without a GPU rasterizes on its CPU. Dropped frames
 * vary from run to run on a busy machine, so --repeat runs each live window that many times and
 * keeps the one with the median share of dropped frames. --update rewrites the baselines from
 * the results. --reference also runs each state in the design prototype and reports how far
 * each app screenshot is from the prototype's.
 */
import { spawn, type ChildProcess } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { createServer as createHttpServer } from 'node:http';
import { createServer } from 'node:net';
import { cpus } from 'node:os';
import { join, resolve } from 'node:path';
import {
	chromium,
	webkit,
	type Browser,
	type BrowserContext,
	type CDPSession,
	type Page
} from '@playwright/test';
import { ALLOWED, EXCEPTIONS } from './motion-lint.ts';
import { probe, type ProbeConfig, type Seen } from '../tests/matrix/probe.ts';
import {
	STATES,
	cells as allCells,
	type Cell,
	type Engine,
	type Net,
	type Step,
	type Viewport
} from '../tests/matrix/states.ts';

const WEB = resolve(import.meta.dirname, '..');
/** the moment `coeus web --fixture instrument`'s timeline starts */
const START = 1_794_927_600_000;
const IDLE = 2;
const MiB = 1_048_576;

type Target = 'app' | 'reference';

function args() {
	const a = process.argv.slice(2);
	const get = (k: string) => {
		const i = a.indexOf(k);
		return i >= 0 ? a[i + 1] : undefined;
	};
	const only = new Map<string, Set<string>>();
	a.forEach((x, i) => {
		if (a[i - 1] !== '--only') return;
		const [k, v] = x.split('=');
		only.set(k, new Set(v.split(',')));
	});
	const reference = get('--reference');
	return {
		only,
		engines: (get('--engines')?.split(',') ?? ['chromium', 'webkit']) as Engine[],
		workers: Number(get('--workers') ?? Math.min(4, Math.max(1, cpus().length >> 1))),
		window: Number(get('--window') ?? 10),
		repeat: Number(get('--repeat') ?? 1),
		out: resolve(get('--out') ?? join(WEB, 'test-results/matrix')),
		trace: a.includes('--trace'),
		reference: reference ? resolve(reference) : null,
		webDir: resolve(get('--web-dir') ?? join(WEB, 'build')),
		coeus: resolve(get('--coeus') ?? join(WEB, '../target/release/coeus')),
		from: get('--from'),
		timings: !a.includes('--no-timings'),
		gate: !a.includes('--no-gate'),
		update: a.includes('--update')
	};
}
const opt = args();

// ---------------------------------------------------------------------------------------
// what a cell records

/** what the page cost over one window of time */
interface Window {
	seconds: number;
	/** stream messages the app received */
	messages: number;
	layouts: number | null;
	recalcs: number | null;
	paints: number | null;
	/** paint area over the window, in viewports */
	painted: number | null;
	/** main-thread busy time, ms per second */
	cpu: number | null;
	frames: { presented: number; dropped: number; partial: number } | null;
	/** frame intervals from the page's own frame callbacks, ms */
	intervals: { p50: number; p95: number; max: number } | null;
	longTasks: number[] | null;
	layers: number | null;
	nodes: number | null;
	/**
	 * DOM node growth while the state holds still, as a fraction: over the idle window, and over
	 * a rest after the live window once its updates have settled
	 */
	growth: number | null;
	listeners: number | null;
	heapMB: number | null;
	gpuMB: number | null;
}

interface Result {
	id: string;
	engine: Engine;
	target: Target;
	screen: string;
	state: string;
	theme: string;
	viewport: Viewport;
	motion: string;
	/** `ok`, or why the state could not be reached */
	outcome: string;
	shot: { file: string; sha256: string } | null;
	/** long tasks while the page loaded and reached the state */
	loading: number[] | null;
	idle: Window | null;
	live: Window | null;
	/** the share of frames dropped in each live window, with --repeat */
	dropped?: number[];
	animations: Record<string, number>;
	/** animations longer than 1 ms, counted under reduced motion */
	lasting: number;
	/** share of pixels that differ from the prototype's screenshot */
	reference?: number;
}

/** what one change costs a page by itself, per change */
interface Calibration {
	changes: number;
	recalcs: number;
	layouts: number;
}

interface Results {
	platform: string;
	browsers: Record<string, string>;
	window: number;
	calibration?: Calibration;
	results: Result[];
}

// ---------------------------------------------------------------------------------------
// servers

function freePort(): Promise<number> {
	return new Promise((ok) => {
		const s = createServer();
		s.listen(0, '127.0.0.1', () => {
			const port = (s.address() as { port: number }).port;
			s.close(() => ok(port));
		});
	});
}

interface Status {
	t: number;
	now: number;
}

/** `coeus web --fixture instrument` over the built app, its clock held */
class Fixture {
	private constructor(
		readonly base: string,
		private proc: ChildProcess
	) {}

	static async start(): Promise<Fixture> {
		const port = await freePort();
		const proc = spawn(
			opt.coeus,
			[
				'web',
				'--fixture',
				'instrument',
				'--open',
				'--web-dir',
				opt.webDir,
				'--listen',
				`127.0.0.1:${port}`,
				'--speed',
				'0'
			],
			{ stdio: ['ignore', 'ignore', 'inherit'] }
		);
		const f = new Fixture(`http://127.0.0.1:${port}`, proc);
		for (let i = 0; ; i++) {
			try {
				await f.seek(0);
				return f;
			} catch (e) {
				if (i > 200 || proc.exitCode !== null) throw new Error(`${opt.coeus} did not start: ${e}`);
				await new Promise((ok) => setTimeout(ok, 50));
			}
		}
	}

	/** moves the clock to `t`, or keeps it where it is, and runs it at `speed` from there */
	async seek(t: number | null, speed = 0): Promise<Status> {
		const at = t === null ? '' : `t=${t}&`;
		const res = await fetch(`${this.base}/fixture/seek?${at}speed=${speed}`, { method: 'POST' });
		if (!res.ok) throw new Error(`seek: ${res.status}`);
		return (await res.json()) as Status;
	}

	stop() {
		this.proc.kill();
	}
}

/**
 * The prototype served from disk. Its Google Fonts stylesheet is answered with the fonts the app
 * ships, so it renders the same faces without a network.
 */
async function servePrototype(dir: string) {
	const fonts = join(WEB, 'node_modules/@fontsource-variable');
	const TYPES: Record<string, string> = {
		html: 'text/html',
		css: 'text/css',
		js: 'text/javascript',
		woff2: 'font/woff2'
	};
	const server = createHttpServer((req, res) => {
		const path = new URL(req.url!, 'http://x').pathname;
		const file = path.startsWith('/__fonts/')
			? join(fonts, path.slice(9))
			: join(dir, path === '/' ? 'index.html' : path);
		if (!existsSync(file)) return res.writeHead(404).end();
		res.writeHead(200, { 'content-type': TYPES[file.split('.').pop()!] ?? '' });
		res.end(readFileSync(file));
	});
	await new Promise<void>((ok) => server.listen(0, '127.0.0.1', ok));
	const base = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
	const face = (family: string, file: string) =>
		`@font-face{font-family:'${family}';font-display:block;font-weight:100 900;font-stretch:75% 100%;src:url(${base}/__fonts/${file}) format('woff2-variations')}`;
	const css =
		face('Instrument Sans', 'instrument-sans/files/instrument-sans-latin-wdth-normal.woff2') +
		face('Geist Mono', 'geist-mono/files/geist-mono-latin-wght-normal.woff2');
	return { base, css, stop: () => server.close() };
}

// ---------------------------------------------------------------------------------------
// Chromium's instruments

// The main-thread tasks of disabled-by-default-devtools.timeline would be ten times the events
// below, and reading them would keep the harness busier than the browser.
const CATEGORIES = [
	'devtools.timeline',
	'blink.user_timing',
	'disabled-by-default-devtools.timeline.frame',
	'disabled-by-default-memory-infra'
];

const MARK = 'matrix-window';

type TraceEvent = {
	name: string;
	ph: string;
	pid: number;
	tid: number;
	ts: number;
	dur?: number;
	args?: any;
};

class Instruments {
	private events: TraceEvent[] = [];
	private metrics: Record<string, number> = {};
	layers = 0;
	private maxLayers = 0;

	private constructor(
		private cdp: CDPSession,
		private viewport: { width: number; height: number }
	) {}

	static async attach(page: Page): Promise<Instruments> {
		const cdp = await page.context().newCDPSession(page);
		const i = new Instruments(cdp, page.viewportSize()!);
		cdp.on('LayerTree.layerTreeDidChange', (e) => {
			if (!e.layers) return;
			i.layers = e.layers.length;
			i.maxLayers = Math.max(i.maxLayers, i.layers);
		});
		await cdp.send('Performance.enable');
		await cdp.send('LayerTree.enable');
		return i;
	}

	private async read() {
		const { metrics } = await this.cdp.send('Performance.getMetrics');
		return Object.fromEntries(metrics.map((m) => [m.name, m.value]));
	}

	async start() {
		// collected before the window opens, so the collection is not counted in it
		await this.cdp.send('HeapProfiler.collectGarbage');
		this.metrics = await this.read();
		this.maxLayers = this.layers;
		this.events = [];
		this.cdp.on('Tracing.dataCollected', this.collect);
		await this.cdp.send('Tracing.start', {
			transferMode: 'ReportEvents',
			traceConfig: {
				includedCategories: CATEGORIES,
				// dumps only when asked for one
				memoryDumpConfig: { triggers: [] } as unknown as Record<string, string>
			}
		});
		await this.cdp.send('Runtime.evaluate', { expression: `performance.mark('${MARK}')` });
	}

	private collect = (e: { value: object[] }) => this.events.push(...(e.value as TraceEvent[]));

	async stop(seconds: number, before: Seen, after: Seen, keep?: string): Promise<Window> {
		const m0 = this.metrics;
		const m1 = await this.read();
		await this.cdp
			.send('Tracing.requestMemoryDump', { deterministic: true, levelOfDetail: 'detailed' })
			.catch(() => {});
		const done = new Promise((ok) => this.cdp.once('Tracing.tracingComplete', ok));
		await this.cdp.send('Tracing.end');
		await done;
		this.cdp.off('Tracing.dataCollected', this.collect);
		// nodes and heap once garbage is gone, so they count what the page holds. The memory dump
		// keeps nodes the page already let go of alive for a moment after it, as many as a screen
		// that a job finishing replaced, so the garbage is collected once the dump is done with them
		await new Promise((ok) => setTimeout(ok, 500));
		await this.cdp.send('HeapProfiler.collectGarbage');
		const held = await this.read();
		if (keep) writeFileSync(keep, JSON.stringify({ traceEvents: this.events }));

		const renderer = this.renderer();
		const events = this.events.filter((e) => e.pid === renderer);
		const viewport = this.viewport.width * this.viewport.height;
		const paints = events.filter((e) => e.name === 'Paint' && e.ph === 'X');
		const frames = { presented: 0, dropped: 0, partial: 0 };
		// the frame in flight as tracing starts reports as dropped
		const opened = Math.min(...this.events.filter((e) => e.ts).map((e) => e.ts)) + 50_000;
		for (const e of events) {
			if (e.name !== 'PipelineReporter' || e.ph !== 'b' || e.ts < opened) continue;
			const state = e.args?.frame_reporter?.state;
			if (state === 'STATE_PRESENTED_ALL') frames.presented++;
			else if (state === 'STATE_DROPPED') frames.dropped++;
			else if (state === 'STATE_PRESENTED_PARTIAL') frames.partial++;
		}
		const d = (k: string) => (m1[k] ?? 0) - (m0[k] ?? 0);
		return {
			seconds,
			messages: after.messages - before.messages,
			layouts: d('LayoutCount'),
			recalcs: d('RecalcStyleCount'),
			paints: paints.length,
			painted: round(
				paints.reduce((s, e) => s + area(e.args?.data?.clip, this.viewport), 0) / viewport,
				2
			),
			cpu: round((d('TaskDuration') * 1000) / seconds, 1),
			frames,
			intervals: null,
			longTasks: after.longTasks?.slice(before.longTasks?.length ?? 0) ?? null,
			layers: this.maxLayers,
			nodes: held.Nodes,
			growth: m0.Nodes ? round((held.Nodes - m0.Nodes) / m0.Nodes, 3) : null,
			listeners: held.JSEventListeners,
			heapMB: round(held.JSHeapUsedSize / MiB, 2),
			gpuMB: gpu(this.events, renderer)
		};
	}

	/**
	 * DOM node growth while the page holds still for `seconds`. A leak shows as nodes that pile
	 * up while nothing changes; what an update adds is the update.
	 */
	async growth(seconds: number): Promise<number | null> {
		const nodes = async () => {
			await this.cdp.send('HeapProfiler.collectGarbage');
			return (await this.read()).Nodes;
		};
		const first = await nodes();
		await new Promise((ok) => setTimeout(ok, seconds * 1000));
		const last = await nodes();
		return first ? round((last - first) / first, 3) : null;
	}

	/** the page's renderer process, which left the mark `start` made */
	private renderer(): number {
		return this.events.find((e) => e.name === MARK)?.pid ?? -1;
	}
}

/** the share of frames that were due and dropped, or 0 where frames were not counted */
const dropped = (w: Window) =>
	w.frames
		? round(
				w.frames.dropped / Math.max(1, w.frames.presented + w.frames.dropped + w.frames.partial),
				4
			)
		: 0;

const round = (x: number, places: number) => Math.round(x * 10 ** places) / 10 ** places;

/** the part of a paint's clip quad inside the viewport, in CSS pixels */
function area(clip: number[] | undefined, v: { width: number; height: number }) {
	if (!clip) return 0;
	const xs = [clip[0], clip[2], clip[4], clip[6]];
	const ys = [clip[1], clip[3], clip[5], clip[7]];
	const w = Math.min(v.width, Math.max(...xs)) - Math.max(0, Math.min(...xs));
	const h = Math.min(v.height, Math.max(...ys)) - Math.max(0, Math.min(...ys));
	return Math.max(0, w) * Math.max(0, h);
}

/**
 * The page's GPU memory from the memory dump: its compositor's tiles and images, and its
 * command and transfer buffers. The GPU process's own memory serves every page of the browser,
 * so it is left out. Null where Chromium dumps nothing for the page.
 */
function gpu(events: TraceEvent[], renderer: number): number | null {
	let bytes: number | null = null;
	for (const e of events) {
		if (e.ph !== 'v' || e.pid !== renderer) continue;
		for (const name of ['cc', 'gpu']) {
			const size = e.args?.dumps?.allocators?.[name]?.attrs?.size?.value;
			if (size !== undefined) bytes = (bytes ?? 0) + parseInt(size, 16);
		}
	}
	return bytes === null ? null : round(bytes / MiB, 1);
}

// ---------------------------------------------------------------------------------------
// running a cell

const VIEWPORTS = {
	phone: {
		viewport: { width: 390, height: 844 },
		deviceScaleFactor: 3,
		isMobile: true,
		hasTouch: true
	},
	desktop: { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 }
} as const;

class Unreached extends Error {}

async function context(browser: Browser, cell: Cell): Promise<BrowserContext> {
	return browser.newContext({
		...VIEWPORTS[cell.viewport],
		colorScheme: cell.theme,
		reducedMotion: cell.motion === 'reduced' ? 'reduce' : 'no-preference',
		// a service worker would answer requests before the routes below see them
		serviceWorkers: 'block',
		locale: 'en-US',
		timezoneId: 'UTC'
	});
}

async function route(ctx: BrowserContext, net: Net) {
	await ctx.route(net.url, async (r) => {
		if ('hold' in net) return;
		if ('abort' in net) return r.abort('connectionrefused');
		const body = net.file ? readFileSync(join(opt.webDir, net.file)) : (net.body ?? '');
		await r.fulfill({ status: net.status, headers: net.headers, body });
	});
}

const seen = (page: Page) => page.evaluate(() => structuredClone(window.__matrix.seen));

/**
 * How long every stream must have sent nothing before the page counts as at rest. A server
 * sends what it holds back to back, a job's log in several batches, so the last of them is
 * the one followed by this much silence while the fleet's clock is held.
 */
const QUIET = 250;

/**
 * Waits for fonts, the stream's first message, and then for the page to come to rest: no
 * stream message for QUIET ms, nothing the page marks `aria-busy` (a log loading or settling,
 * a sheet rising), and no finite animation running.
 */
async function settle(page: Page, streamed: boolean) {
	await page.evaluate(() => document.fonts.ready.then(() => undefined));
	if (streamed) {
		await page
			.waitForFunction(
				() => {
					const s = window.__matrix.seen;
					return s.messages > 0 || (performance.now() > 800 && s.streams === 0);
				},
				undefined,
				{ timeout: 3000, polling: 50 }
			)
			.catch(() => {});
	}
	await page
		.waitForFunction(
			(quiet) =>
				performance.now() - window.__matrix.seen.last >= quiet &&
				!document.querySelector('[aria-busy="true"]') &&
				!document
					.getAnimations()
					.some(
						(a) =>
							a.playState === 'running' &&
							Number.isFinite(Number(a.effect?.getComputedTiming().endTime))
					),
			QUIET,
			{ timeout: 5000, polling: 50 }
		)
		.catch(() => {});
	await page.evaluate(
		() => new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok)))
	);
}

interface Run {
	cell: Cell;
	engine: Engine;
	target: Target;
}

class Worker {
	constructor(
		private browsers: Record<Engine, Browser>,
		private fixture: Fixture | null,
		private prototype: { base: string; css: string } | null
	) {}

	/** a run that fails for any reason but an unreached state gets one more try */
	async run(r: Run): Promise<Result> {
		try {
			return await this.attempt(r);
		} catch {
			try {
				return await this.attempt(r);
			} catch (e) {
				const failed = blank(r);
				failed.outcome = `error: ${e instanceof Error ? e.message.split('\n')[0] : e}`;
				return failed;
			}
		}
	}

	private async attempt({ cell, engine, target }: Run): Promise<Result> {
		const { state } = cell;
		const result = blank({ cell, engine, target });
		const name = `${cell.id}.${engine}${target === 'app' ? '' : '.reference'}`;
		const animations = (s: Seen) => {
			for (const [k, n] of Object.entries(s.animations))
				result.animations[k] = (result.animations[k] ?? 0) + n;
			if (cell.motion === 'reduced') result.lasting += s.lasting;
		};

		// the state, photographed, then 2 s at rest
		const ctx = await context(this.browsers[engine], cell);
		try {
			const page = await this.open(ctx, run(cell, engine, target), state.t, false);
			const instruments = engine === 'chromium' ? await Instruments.attach(page) : null;
			await this.steps(page, ctx, { cell, engine, target }, state.t);
			const file = join(opt.out, 'shots', `${name}.png`);
			// endless animations stand at their start, so the picture is the same every run
			const png = await page.screenshot({ path: file, caret: 'hide', animations: 'disabled' });
			result.shot = {
				file: `shots/${name}.png`,
				sha256: createHash('sha256').update(png).digest('hex')
			};
			// the screenshot stops endless animations and they start over once it is taken, which
			// belongs to the screenshot, not to the page at rest
			await settle(page, false);
			const before = await seen(page);
			result.loading = before.longTasks;
			if (instruments) {
				await instruments.start();
				await page.waitForTimeout(IDLE * 1000);
				const after = await seen(page);
				result.idle = await instruments.stop(IDLE, before, after, this.keep(`${name}.idle`));
			}
			animations(await seen(page));
		} catch (e) {
			if (!(e instanceof Unreached)) throw e;
			result.outcome = `unreached: ${e.message}`;
			return result;
		} finally {
			await ctx.close();
		}

		// windows of live updates, from where the state starts to move
		if (state.live === undefined || cell.motion !== 'full') return result;
		const windows: Window[] = [];
		for (let i = 0; i < opt.repeat; i++) {
			const ctx = await context(this.browsers[engine], cell);
			try {
				const page = await this.open(ctx, run(cell, engine, target), state.live, true);
				const instruments = engine === 'chromium' ? await Instruments.attach(page) : null;
				await settle(page, true);
				const before = await seen(page);
				await instruments?.start();
				if (engine === 'webkit') await page.evaluate(() => window.__matrix.frames(true));
				if (target === 'app') {
					const status = await this.fixture!.seek(state.live, 1);
					await page.evaluate((now) => window.__matrix.run(now), status.now);
				}
				await page.waitForTimeout(opt.window * 1000);
				// held where it ran to: moving it anywhere would be one more change
				if (target === 'app') {
					const held = await this.fixture!.seek(null);
					await page.evaluate((now) => window.__matrix.hold(now), held.now);
				}
				const after = await seen(page);
				const keep = this.keep(`${name}.live${i ? `.${i}` : ''}`);
				const w = instruments
					? await instruments.stop(opt.window, before, after, keep)
					: webkitWindow(opt.window, before, after);
				// the prototype runs its own clock, so it never holds still
				if (instruments) {
					await settle(page, false);
					w.growth = target === 'app' ? await instruments.growth(IDLE) : null;
				}
				windows.push(w);
				animations(after);
			} finally {
				await ctx.close();
			}
		}
		// the window whose dropped frames are the median, with every window's share beside it
		windows.sort((a, b) => dropped(a) - dropped(b));
		result.live = windows[windows.length >> 1];
		result.dropped = windows.map(dropped);
		return result;
	}

	private keep(name: string) {
		return opt.trace ? join(opt.out, 'traces', `${name}.json`) : undefined;
	}

	/** a page at `t` on the timeline, with the state's network answers and stream rewrites */
	private async open(ctx: BrowserContext, r: Run, t: number, live: boolean): Promise<Page> {
		const { state } = r.cell;
		let config: ProbeConfig;
		let url: string;
		if (r.target === 'app') {
			const status = await this.fixture!.seek(t);
			config = { now: status.now, rewrites: state.rewrite ?? [] };
			url = this.fixture!.base + state.path;
		} else {
			const ref = state.reference!;
			const hash = [`s=${ref.s}`, t > 0 && `t=${t}`, `v=${live ? 1 : 0}`, ref.hash]
				.filter(Boolean)
				.join('&');
			config = { now: START, rewrites: [] };
			url = `${this.prototype!.base}/#${hash}`;
			const css = this.prototype!.css;
			await ctx.route('https://fonts.googleapis.com/**', (route) =>
				route.fulfill({ contentType: 'text/css', body: css })
			);
		}
		await ctx.addInitScript(probe, config);
		for (const net of state.net ?? []) await route(ctx, net);
		const page = await ctx.newPage();
		await page.goto(url);
		if (r.target === 'reference')
			for (const go of state.reference!.go ?? []) {
				await settle(page, false);
				await page.evaluate((g) => (window as any).__coeus.go(g), go);
			}
		return page;
	}

	private async steps(page: Page, ctx: BrowserContext, r: Run, t: number) {
		const streamed = !(r.cell.state.net ?? []).some((n) => /stream|\*\*\/api\/\*\*/.test(n.url));
		await settle(page, streamed && r.target === 'app');
		for (const step of r.cell.state.steps ?? []) {
			await this.step(page, ctx, r, step, t);
			if ('advance' in step) t += step.advance;
			await settle(page, false);
		}
	}

	private async step(page: Page, ctx: BrowserContext, r: Run, step: Step, t: number) {
		if ('click' in step) {
			const el = page.locator(step.click, step.text ? { hasText: step.text } : {}).first();
			if (!(await el.count())) throw new Unreached(`nothing matches ${describe(step)}`);
			await el.click();
		} else if ('scroll' in step) {
			const el = page.locator(step.scroll).first();
			if (!(await el.count())) throw new Unreached(`nothing matches ${step.scroll}`);
			await el.evaluate((e, top) => e.scrollTo({ top }), step.top);
		} else if ('advance' in step) {
			if (r.target === 'app') {
				const status = await this.fixture!.seek(t, 1);
				await page.evaluate((now) => window.__matrix.run(now), status.now);
				await page.waitForTimeout(step.advance * 1000);
				const held = await this.fixture!.seek(t + step.advance);
				await page.evaluate((now) => window.__matrix.hold(now), held.now);
				// the page's clock reads the time once a second, so its next tick shows where it held,
				// and the tick after puts plain text back where the last update rolled digits
				await page.waitForTimeout(2100);
			} else {
				// the prototype's speed lives in its developer panel
				await page.keyboard.press('.');
				await page.click('[data-dev="v:1"]');
				await page.waitForTimeout(step.advance * 1000);
				await page.click('[data-dev="v:0"]');
				await page.keyboard.press('.');
			}
		} else if ('reload' in step) await page.reload();
		else if ('net' in step) await route(ctx, step.net);
		else if ('restore' in step) await ctx.unroute(step.restore);
		else await page.waitForTimeout(step.wait);
	}
}

const run = (cell: Cell, engine: Engine, target: Target): Run => ({ cell, engine, target });

const blank = ({ cell, engine, target }: Run): Result => ({
	id: cell.id,
	engine,
	target,
	screen: cell.state.screen,
	state: cell.state.name,
	theme: cell.theme,
	viewport: cell.viewport,
	motion: cell.motion,
	outcome: 'ok',
	shot: null,
	loading: null,
	idle: null,
	live: null,
	animations: {},
	lasting: 0
});
const describe = (step: Step) =>
	'click' in step
		? `${step.click}${step.text ? ` with "${step.text}"` : ''}`
		: JSON.stringify(step);

function webkitWindow(seconds: number, before: Seen, after: Seen): Window {
	const f = [...after.frames].sort((a, b) => a - b);
	const q = (p: number) => f[Math.min(f.length - 1, Math.floor(p * f.length))] ?? 0;
	return {
		seconds,
		messages: after.messages - before.messages,
		layouts: null,
		recalcs: null,
		paints: null,
		painted: null,
		cpu: null,
		frames: null,
		intervals: f.length ? { p50: q(0.5), p95: q(0.95), max: f.at(-1)! } : null,
		longTasks: null,
		layers: null,
		nodes: null,
		growth: null,
		listeners: null,
		heapMB: null,
		gpuMB: null
	};
}

// ---------------------------------------------------------------------------------------
// the calibration: what one change costs by itself

const CHANGES = 20;
const CALIBRATION_RUNS = 3;
const CALIBRATION = `<!doctype html><meta name="viewport" content="width=device-width">
<style>
body{font:16px/1.4 sans-serif;margin:40px}
#n>span{display:inline-block;animation:in 170ms cubic-bezier(0.23,1,0.32,1)}
@keyframes in{from{transform:translateY(0.42em);opacity:0}}
</style>
<p>builds <span id="n"><span>0</span></span> of 140</p>`;

/**
 * The least a change that moves anything costs Chromium in this harness: new text that slides in
 * with one composited 170 ms CSS animation of its transform, as a rolled digit does, twenty
 * times 500 ms apart so each animation ends before the next change. Run alone, before the cells,
 * with the probe the cells run with, the median of a few runs. matrix-budget.json allows each
 * change this much and a margin.
 */
async function calibrate(browser: Browser): Promise<Calibration> {
	const runs: Calibration[] = [];
	for (let i = 0; i < CALIBRATION_RUNS; i++) {
		const ctx = await browser.newContext(VIEWPORTS.desktop);
		try {
			await ctx.addInitScript(probe, { now: START, rewrites: [] });
			await ctx.route('http://calibration.invalid/', (r) =>
				r.fulfill({ contentType: 'text/html', body: CALIBRATION })
			);
			const page = await ctx.newPage();
			await page.goto('http://calibration.invalid/');
			await settle(page, false);
			const instruments = await Instruments.attach(page);
			const before = await seen(page);
			await instruments.start();
			await page.evaluate(
				(n) =>
					new Promise<void>((ok) => {
						const el = document.getElementById('n')!;
						let i = 0;
						const timer = setInterval(() => {
							const digit = document.createElement('span');
							digit.textContent = String(++i);
							el.replaceChildren(digit);
							if (i === n) {
								clearInterval(timer);
								setTimeout(ok, 500);
							}
						}, 500);
					}),
				CHANGES
			);
			const w = await instruments.stop(CHANGES / 2, before, await seen(page));
			runs.push({
				changes: CHANGES,
				recalcs: round(w.recalcs! / CHANGES, 2),
				layouts: round(w.layouts! / CHANGES, 2)
			});
		} finally {
			await ctx.close();
		}
	}
	const median = (k: 'recalcs' | 'layouts') =>
		runs.map((r) => r[k]).sort((a, b) => a - b)[runs.length >> 1];
	return { changes: CHANGES, recalcs: median('recalcs'), layouts: median('layouts') };
}

// ---------------------------------------------------------------------------------------
// the reference: how far each app screenshot is from the prototype's

async function compare(browser: Browser, a: string, b: string, out: string): Promise<number> {
	const page = await browser.newPage();
	try {
		const url = (f: string) => `data:image/png;base64,${readFileSync(f).toString('base64')}`;
		const { share, png } = await page.evaluate(
			async ([a, b]) => {
				const load = async (src: string) => {
					const img = new Image();
					img.src = src;
					await img.decode();
					return img;
				};
				const [x, y] = await Promise.all([load(a), load(b)]);
				const w = Math.max(x.width, y.width);
				const h = Math.max(x.height, y.height);
				const canvas = new OffscreenCanvas(w, h);
				const g = canvas.getContext('2d')!;
				g.drawImage(x, 0, 0);
				const pa = g.getImageData(0, 0, w, h).data;
				g.clearRect(0, 0, w, h);
				g.drawImage(y, 0, 0);
				const pb = g.getImageData(0, 0, w, h).data;
				const diff = g.createImageData(w, h);
				let n = 0;
				for (let i = 0; i < pa.length; i += 4) {
					const d = Math.max(
						Math.abs(pa[i] - pb[i]),
						Math.abs(pa[i + 1] - pb[i + 1]),
						Math.abs(pa[i + 2] - pb[i + 2]),
						Math.abs(pa[i + 3] - pb[i + 3])
					);
					const grey = (pa[i] + pa[i + 1] + pa[i + 2]) / 3;
					if (d > 24) n++;
					diff.data.set(d > 24 ? [230, 40, 40, 255] : [grey, grey, grey, 70], i);
				}
				g.putImageData(diff, 0, 0);
				const blob = await canvas.convertToBlob();
				const bytes = new Uint8Array(await blob.arrayBuffer());
				let bin = '';
				for (const byte of bytes) bin += String.fromCharCode(byte);
				return { share: n / (w * h), png: btoa(bin) };
			},
			[url(a), url(b)]
		);
		writeFileSync(out, Buffer.from(png, 'base64'));
		return round(share, 4);
	} finally {
		await page.close();
	}
}

// ---------------------------------------------------------------------------------------
// the gate

interface Budget {
	counts: {
		longTasks: number;
		layouts: { perMessage: number; perSecond: number };
		recalcs: { perMessage: number; perSecond: number };
		idlePaintsPerSecond: number;
		layers: Record<Viewport, number>;
		nodes: number;
		growth: number;
		heapMB: number;
	};
	timings: { dropped: number; gpuMB: number };
}

const BASELINES = join(WEB, 'tests/matrix/baselines');
const platform = () => `${process.platform}-${process.arch}`;
const key = (r: Result) => `${r.id}.${r.engine}`;

/** the motion rule of scripts/motion-lint.ts, applied to an animation the page ran */
function composited(animation: string) {
	const [, property, at] = animation.split('|');
	if (ALLOWED.has(property)) return true;
	return EXCEPTIONS.some(
		(e) =>
			e.property === property &&
			(e.at.startsWith('@keyframes')
				? e.at === at
				: e.at.startsWith('.') && at.split('.').includes(e.at.slice(1)))
	);
}

function gate(all: Results, timings: boolean): string[] {
	const budget = JSON.parse(readFileSync(join(WEB, 'matrix-budget.json'), 'utf8')) as Budget;
	const c = budget.counts;
	const file = join(BASELINES, `${all.platform}.json`);
	const baselines: Record<string, string> | null = existsSync(file)
		? JSON.parse(readFileSync(file, 'utf8'))
		: null;
	const problems: string[] = [];
	for (const r of all.results.filter((r) => r.target === 'app')) {
		const fail = (what: string) => problems.push(`${key(r)}: ${what}`);
		if (r.outcome.startsWith('error')) fail(r.outcome);
		if (baselines) {
			const want = baselines[key(r)];
			const got = r.shot?.sha256 ?? r.outcome;
			if (want === undefined) fail('no baseline');
			else if (want !== got)
				fail(r.outcome === 'ok' ? `screenshot differs from its baseline` : r.outcome);
		}
		for (const a of Object.keys(r.animations)) if (!composited(a)) fail(`animates ${a}`);
		if (r.lasting) fail(`${r.lasting} animation(s) longer than 1 ms under reduced motion`);
		for (const [name, w] of [
			['idle', r.idle],
			['live', r.live]
		] as const) {
			if (!w) continue;
			const over = (what: string, value: number | null, limit: number) => {
				if (value !== null && value > limit) fail(`${name}: ${what} ${value} > ${limit}`);
			};
			// a long task is 50 ms of wall time, so a builder busy with another job at the same
			// time fails cells whose page stays within budget alone: build the matrix by itself
			over('long tasks', w.longTasks?.length ?? null, c.longTasks);
			over('layers', w.layers, c.layers[r.viewport]);
			over('DOM nodes', w.nodes, c.nodes);
			over('heap MB', w.heapMB, c.heapMB);
			const per = (b: { perMessage: number; perSecond: number }) =>
				b.perMessage * w.messages + b.perSecond * w.seconds;
			over('layouts', w.layouts, per(c.layouts));
			over('style recalcs', w.recalcs, per(c.recalcs));
			if (name === 'idle') over('paints', w.paints, c.idlePaintsPerSecond * w.seconds);
			over('DOM growth', w.growth, c.growth);
			if (timings && w.frames) over('dropped frames', dropped(w), budget.timings.dropped);
			if (timings) over('GPU MB', w.gpuMB, budget.timings.gpuMB);
		}
	}
	return problems;
}

// ---------------------------------------------------------------------------------------
// the report

function report(all: Results) {
	const cols = [
		'cell',
		'engine',
		'target',
		'out',
		'msgs',
		'lay',
		'rcl',
		'pnt',
		'area',
		'cpu',
		'drop',
		'p95',
		'lt',
		'lyr',
		'nodes',
		'heap',
		'gpu',
		'ref'
	];
	const rows = [cols];
	const n = (x: number | null | undefined) => (x === null || x === undefined ? '-' : String(x));
	for (const r of all.results) {
		for (const [name, w] of [
			['idle', r.idle],
			['live', r.live]
		] as const) {
			if (name === 'live' && !w) continue;
			const f = w?.frames;
			rows.push([
				`${r.id}${name === 'live' ? ' (live)' : ''}`,
				r.engine,
				r.target,
				r.outcome.split(':')[0],
				n(w?.messages),
				n(w?.layouts),
				n(w?.recalcs),
				n(w?.paints),
				n(w?.painted),
				n(w?.cpu),
				f ? `${f.dropped}/${f.presented + f.dropped + f.partial}` : '-',
				n(w?.intervals?.p95),
				n(w?.longTasks?.length),
				n(w?.layers),
				n(w?.nodes),
				n(w?.heapMB),
				n(w?.gpuMB),
				n(r.reference)
			]);
		}
	}
	const widths = cols.map((_, i) => Math.max(...rows.map((r) => r[i].length)));
	for (const row of rows) console.log(row.map((x, i) => x.padEnd(widths[i])).join('  '));
}

// ---------------------------------------------------------------------------------------

async function main(): Promise<Results> {
	const pinned = process.env.PLAYWRIGHT_VERSION;
	const ours = JSON.parse(
		readFileSync(join(WEB, 'node_modules/@playwright/test/package.json'), 'utf8')
	).version;
	if (pinned && pinned !== ours)
		throw new Error(`@playwright/test ${ours} drives browsers built for Playwright ${pinned}`);

	const pick = (k: string, v: string) => !opt.only.has(k) || opt.only.get(k)!.has(v);
	const cells = allCells(STATES).filter(
		(c) =>
			pick('screen', c.state.screen) &&
			pick('state', c.state.name) &&
			pick('theme', c.theme) &&
			pick('viewport', c.viewport) &&
			pick('motion', c.motion)
	);
	const targets = (['app', 'reference'] as Target[]).filter(
		(t) => pick('target', t) && (t === 'app' || opt.reference)
	);
	const runs = cells.flatMap((cell) =>
		opt.engines
			.filter((e) => pick('engine', e) && (e === 'chromium' || cell.viewport === 'phone'))
			.flatMap((engine) =>
				targets
					.filter((t) => t === 'app' || cell.state.reference)
					.map((target) => run(cell, engine, target))
			)
	);
	mkdirSync(join(opt.out, 'shots'), { recursive: true });
	if (opt.trace) mkdirSync(join(opt.out, 'traces'), { recursive: true });

	// tracing is one per browser, so each worker drives browsers of its own
	const launch = async (): Promise<Record<Engine, Browser>> => ({
		chromium: await chromium.launch({
			channel: 'chromium',
			// chromium redraws only the changed part of a tile, and skia clips a curve's edges to
			// the area it redraws, so a rounded corner's antialiasing would depend on which change
			// last touched its tile, which depends on when each frame landed. whole-tile redraws make
			// a screenshot depend only on the page, which the hashed linux screenshots need
			args: process.platform === 'linux' ? ['--disable-partial-raster'] : []
		}),
		webkit: opt.engines.includes('webkit') ? await webkit.launch() : (null as unknown as Browser)
	});
	const browsers = await Promise.all(Array.from({ length: opt.workers }, launch));
	const prototype = opt.reference ? await servePrototype(opt.reference) : null;
	const fixtures = targets.includes('app')
		? await Promise.all(Array.from({ length: opt.workers }, () => Fixture.start()))
		: [];
	const versions = Object.fromEntries(
		Object.entries(browsers[0])
			.filter(([, b]) => b)
			.map(([name, b]) => [name, b.version()])
	);
	const results: Result[] = [];
	let calibration: Calibration | undefined;
	try {
		calibration = await calibrate(browsers[0].chromium);
		const queue = [...runs];
		let done = 0;
		await Promise.all(
			browsers.map(async (mine, i) => {
				const worker = new Worker(mine, fixtures[i] ?? null, prototype);
				for (let r = queue.shift(); r; r = queue.shift()) {
					results.push(await worker.run(r));
					done++;
					if (done % 20 === 0 || done === runs.length)
						console.error(`matrix: ${done}/${runs.length}`);
				}
			})
		);
		for (const r of results.filter((r) => r.target === 'app' && r.shot)) {
			const ref = results.find(
				(x) => x.target === 'reference' && x.id === r.id && x.engine === r.engine && x.shot
			);
			if (!ref) continue;
			const out = join(opt.out, 'shots', `${r.id}.${r.engine}.diff.png`);
			r.reference = await compare(
				browsers[0].chromium,
				join(opt.out, r.shot!.file),
				join(opt.out, ref.shot!.file),
				out
			);
		}
	} finally {
		for (const f of fixtures) f.stop();
		prototype?.stop();
		for (const b of browsers) {
			await b.chromium.close();
			await b.webkit?.close();
		}
	}
	const order = (r: Result) => `${r.id}.${r.engine}.${r.target}`;
	results.sort((a, b) => order(a).localeCompare(order(b)));
	return {
		platform: platform(),
		browsers: versions,
		window: opt.window,
		calibration,
		results
	};
}

const all: Results = opt.from ? JSON.parse(readFileSync(opt.from, 'utf8')) : await main();
if (!opt.from) {
	writeFileSync(join(opt.out, 'matrix-results.json'), JSON.stringify(all, null, '\t') + '\n');
	report(all);
}
if (opt.update) {
	mkdirSync(BASELINES, { recursive: true });
	const file = join(BASELINES, `${all.platform}.json`);
	const old = existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : {};
	for (const r of all.results.filter((r) => r.target === 'app' && !r.outcome.startsWith('error')))
		old[key(r)] = r.shot?.sha256 ?? r.outcome;
	const sorted = Object.fromEntries(Object.entries(old).sort(([a], [b]) => a.localeCompare(b)));
	writeFileSync(file, JSON.stringify(sorted, null, '\t') + '\n');
	console.error(`matrix: wrote ${file}`);
}
if (opt.gate || opt.from) {
	if (all.calibration) {
		const { recalcs, layouts } = all.calibration;
		const c = (JSON.parse(readFileSync(join(WEB, 'matrix-budget.json'), 'utf8')) as Budget).counts;
		console.error(
			`matrix: one composited animation per change costs ${recalcs} style recalcs and ${layouts} layouts here; a message may cost ${c.recalcs.perMessage} and ${c.layouts.perMessage}`
		);
	}
	const problems = gate(all, opt.timings);
	if (problems.length) {
		console.error(problems.join('\n'));
		console.error(`\nmatrix: ${problems.length} problem(s)`);
		process.exit(1);
	}
	console.error(
		`matrix: ${all.results.filter((r) => r.target === 'app').length} results within budget`
	);
}
