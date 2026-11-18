/**
 * Writes a `.br` (quality 11) and a `.gz` (level 9) beside every text file in build/, so
 * the server sends bytes compressed once at their smallest instead of compressing each
 * response at a fast, larger setting. Runs after `vite build`; the service worker's
 * precache list is already written by then, so it never lists these copies.
 *
 * Then writes `<file>.etag`, a quoted content hash, beside every file outside
 * `_app/immutable/`, compressed copies included, for the server to send as the ETag: files
 * in the Nix store all share one mtime, so a validator built from mtime and size would call
 * a new `index.html` of the same size unmodified.
 */
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join, resolve, sep } from 'node:path';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';

const BUILD = resolve(import.meta.dirname, '../build');
const TEXT = /\.(js|css|html|svg|json|webmanifest|txt)$/;

function* files(dir: string): Generator<string> {
	for (const name of readdirSync(dir)) {
		const path = join(dir, name);
		if (statSync(path).isDirectory()) yield* files(path);
		else yield path;
	}
}

let raw = 0;
let br = 0;
for (const path of [...files(BUILD)].filter((p) => TEXT.test(p))) {
	const data = readFileSync(path);
	const b = brotliCompressSync(data, {
		params: {
			[constants.BROTLI_PARAM_QUALITY]: 11,
			[constants.BROTLI_PARAM_SIZE_HINT]: data.length
		}
	});
	writeFileSync(`${path}.br`, b);
	writeFileSync(`${path}.gz`, gzipSync(data, { level: 9 }));
	raw += data.length;
	br += b.length;
}
console.log(
	`compressed build/: ${(raw / 1024).toFixed(0)} KiB of text, ${(br / 1024).toFixed(0)} KiB as br`
);

const IMMUTABLE = join(BUILD, '_app', 'immutable') + sep;
let etags = 0;
for (const path of [...files(BUILD)].filter((p) => !p.startsWith(IMMUTABLE))) {
	const hash = createHash('sha256').update(readFileSync(path)).digest('base64url');
	writeFileSync(`${path}.etag`, `"${hash.slice(0, 24)}"`);
	etags++;
}
console.log(`wrote ${etags} ETag sidecars`);
