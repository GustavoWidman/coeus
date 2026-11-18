/**
 * Fails when a route's cold path ships more JS or CSS than perf-budget.json allows. A route's
 * cold path is what loads before its first screen renders: the entry index.html loads, the
 * module that draws the route, and every module either imports statically, with their
 * stylesheets, each gzipped at level 9. Vite's manifest names them. The lazy imports a screen
 * makes once it is up load after first paint and are left out.
 *
 *   bun scripts/budget.ts            (after `bun run build`)
 */
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { gzipSync } from 'node:zlib';

const BUILD = resolve(import.meta.dirname, '../build');

type Limits = { js: number; css: number };
type Budget = {
	default: Limits;
	routes: Record<string, Partial<Limits> & { module: string }>;
};
type Chunk = { file: string; isEntry?: boolean; imports?: string[]; css?: string[] };

const budget: Budget = JSON.parse(
	readFileSync(resolve(import.meta.dirname, '../perf-budget.json'), 'utf8')
);
const manifest: Record<string, Chunk> = JSON.parse(
	readFileSync(join(BUILD, '.vite/manifest.json'), 'utf8')
);
const entry = 'index.html';
if (!manifest[entry]) throw new Error('the build has no index.html entry');

/** the files `keys` load before they run */
function cold(keys: string[]) {
	const seen = new Set<string>();
	const files = { js: new Set<string>(), css: new Set<string>() };
	const visit = (key: string) => {
		if (seen.has(key)) return;
		seen.add(key);
		const chunk = manifest[key];
		if (!chunk) throw new Error(`the build made nothing of ${key}`);
		files.js.add(chunk.file);
		for (const css of chunk.css ?? []) files.css.add(css);
		for (const i of chunk.imports ?? []) visit(i);
	};
	keys.forEach(visit);
	return files;
}

const size = (file: string) => gzipSync(readFileSync(join(BUILD, file)), { level: 9 }).length;
const kib = (n: number) => (n / 1024).toFixed(1).padStart(6);
let over = 0;
for (const [route, limits] of Object.entries(budget.routes)) {
	const files = cold([entry, limits.module]);
	console.log(`${route}`);
	for (const kind of ['js', 'css'] as const) {
		const limit = limits[kind] ?? budget.default[kind];
		let total = 0;
		for (const file of files[kind]) {
			const n = size(file);
			total += n;
			console.log(`  ${kib(n)} KiB  ${file}`);
		}
		const bad = total / 1024 > limit;
		over += +bad;
		console.log(`  ${kib(total)} KiB  ${kind} total, budget ${limit} KiB${bad ? '  OVER' : ''}`);
	}
}
if (over) {
	console.error('\nOver budget. Shrink the cold path, or raise its line in perf-budget.json.');
	process.exit(1);
}
