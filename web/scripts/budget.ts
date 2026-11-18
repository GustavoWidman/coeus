/**
 * Fails when the app's cold path ships more JS or CSS than perf-budget.json allows. The cold
 * path is what the built index.html loads before the first screen renders: its module
 * scripts, the modules they preload and its stylesheets, each gzipped at level 9. Every
 * screen renders from that one entry, so one line covers them all; the app's lazy imports
 * load after first paint and are left out.
 *
 *   bun scripts/budget.ts            (after `bun run build`)
 */
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { gzipSync } from 'node:zlib';

const BUILD = resolve(import.meta.dirname, '../build');

type Limits = { js: number; css: number };
const budget: Limits = JSON.parse(
	readFileSync(resolve(import.meta.dirname, '../perf-budget.json'), 'utf8')
);

const html = readFileSync(join(BUILD, 'index.html'), 'utf8');
const tags = [...html.matchAll(/<(?:script|link)\b[^>]*>/g)].map((m) => m[0]);
const attr = (tag: string, name: string) =>
	new RegExp(`\\s${name}="([^"]*)"`).exec(tag)?.[1] ?? null;
/** The URL a tag loads on the cold path, if it loads one. */
function load(tag: string): { kind: 'js' | 'css'; url: string } | null {
	const rel = attr(tag, 'rel');
	const url = tag.startsWith('<script')
		? attr(tag, 'type') === 'module' && attr(tag, 'src')
		: rel === 'modulepreload' || rel === 'stylesheet'
			? attr(tag, 'href')
			: null;
	return url ? { kind: rel === 'stylesheet' ? 'css' : 'js', url } : null;
}
const files = { js: [] as string[], css: [] as string[] };
for (const tag of tags) {
	const l = load(tag);
	if (l) files[l.kind].push(l.url);
}
if (!files.js.length) throw new Error('build/index.html loads no module script');

const kib = (n: number) => (n / 1024).toFixed(1).padStart(6);
let over = 0;
for (const kind of ['js', 'css'] as const) {
	console.log(`${kind}`);
	let total = 0;
	for (const url of files[kind]) {
		const size = gzipSync(readFileSync(join(BUILD, url)), { level: 9 }).length;
		total += size;
		console.log(`  ${kib(size)} KiB  ${url}`);
	}
	const bad = total / 1024 > budget[kind];
	over += +bad;
	console.log(`  ${kib(total)} KiB  total, budget ${budget[kind]} KiB${bad ? '  OVER' : ''}`);
}
if (over) {
	console.error('\nOver budget. Shrink the cold path, or raise its line in perf-budget.json.');
	process.exit(1);
}
