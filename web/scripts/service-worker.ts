/**
 * The build's half of the service worker: the offline page, made from the offline Worker's own
 * files as the Worker's module makes it (nix/offline-worker.nix), and the list of every file
 * the build wrote, spliced into service-worker.js with a version that changes whenever any of
 * them does.
 */
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import type { Plugin } from 'vite';

const WEB = resolve(import.meta.dirname, '..');
const WORKER = resolve(WEB, '../worker');
const read = (path: string) => readFileSync(path, 'utf8');
const sha256 = (text: string) => `'sha256-${createHash('sha256').update(text).digest('base64')}'`;

/**
 * The offline page without a host list: the build cannot know which hosts serve it, so the
 * page's stylesheet hides the empty list. Its policy goes in a meta, as nothing sends it.
 */
export function offlinePage(): string {
	const style = read(join(WEB, 'src/lib/ui/tokens.css')) + read(join(WORKER, 'offline.css'));
	const script = read(join(WORKER, 'offline.js'));
	const icon =
		'data:image/svg+xml,' +
		read(join(WEB, 'static/favicon.svg'))
			.replace(/#/g, '%23')
			.replace(/</g, '%3C')
			.replace(/>/g, '%3E')
			.replace(/"/g, "'")
			.replace(/\n/g, '');
	const csp = [
		"default-src 'none'",
		`style-src ${sha256(style)}`,
		`script-src ${sha256(script)}`,
		'img-src data:',
		"connect-src 'self'",
		"base-uri 'none'",
		"form-action 'none'"
	].join('; ');
	const fill: Record<string, string> = {
		'@style@': `<style>${style}</style>`,
		'@script@': `<script>${script}</script>`,
		'@icon@': icon,
		'@count@': '0',
		'@hosts@': ''
	};
	return read(join(WORKER, 'offline.html'))
		.replace(/@(?:style|script|icon|count|hosts)@/g, (k) => fill[k])
		.replace(
			'<meta charset="utf-8" />',
			`<meta charset="utf-8" />\n\t\t<meta http-equiv="Content-Security-Policy" content="${csp}" />`
		);
}

function files(dir: string): string[] {
	return readdirSync(dir).flatMap((name) => {
		const path = join(dir, name);
		return statSync(path).isDirectory() ? files(path) : [path];
	});
}

export const SERVICE_WORKER = 'service-worker.js';

/** writes the offline page into the build, then tells the service worker every file it made */
export function serviceWorker(): Plugin {
	let out = '';
	return {
		name: 'coeus-service-worker',
		apply: 'build',
		configResolved(config) {
			out = resolve(config.root, config.build.outDir);
		},
		closeBundle() {
			writeFileSync(join(out, 'offline.html'), offlinePage());
			const version = createHash('sha256');
			const precache: string[] = [];
			for (const path of files(out).sort()) {
				const name = relative(out, path).split('\\').join('/');
				if (name === SERVICE_WORKER || name.startsWith('.vite/')) continue;
				version.update(name).update(readFileSync(path));
				// the shell is cached as `/`, which is how every navigation asks for it
				if (name !== 'index.html') precache.push(`/${name}`);
			}
			const sw = join(out, SERVICE_WORKER);
			const source = read(sw)
				.replace(/(["'`])__COEUS_BUILD__\1/, JSON.stringify(version.digest('hex').slice(0, 12)))
				.replace(/(["'`])__COEUS_FILES__\1/, JSON.stringify(JSON.stringify(precache)));
			if (source.includes('__COEUS_')) throw new Error(`${SERVICE_WORKER}: a placeholder is left`);
			// it registers as a classic script, which cannot load a chunk it shares with the app
			if (/(?:^|[;}\s])import\s*[{*"']/.test(source))
				throw new Error(`${SERVICE_WORKER}: imports a module the app also imports`);
			writeFileSync(sw, source);
		}
	};
}
