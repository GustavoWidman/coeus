/**
 * Draws the coeus mark and renders every icon file from it.
 *
 *   bun scripts/icons.ts            (from web/)
 *
 * The mark is the wordmark's ring (`.wm::before` in app.css) drawn large: a ring, a gap and
 * a center dot, in the light background color on an ink tile. Rasterizing goes through
 * Chromium; set CHROMIUM to a browser binary when the Playwright-managed one is not
 * installed.
 */
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { chromium } from '@playwright/test';

const root = resolve(import.meta.dirname, '..');
/** --ink and --bg of the light theme. */
const INK = '#141413';
const BG = '#f6f6f3';

/**
 * The tile, with the ring's outer diameter at `size` of it. In the wordmark's 9 px box the
 * ring's border is 1.4 px and the dot's radius 1.1 px.
 */
function icon({ corner, size }: { corner: number; size: number }) {
	const u = (512 * size) / 9;
	const n = (v: number) => +v.toFixed(2);
	return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><rect width="512" height="512" rx="${corner}" fill="${INK}"/><circle cx="256" cy="256" r="${n(3.8 * u)}" fill="none" stroke="${BG}" stroke-width="${n(1.4 * u)}"/><circle cx="256" cy="256" r="${n(1.1 * u)}" fill="${BG}"/></svg>\n`;
}

const ROUNDED = 112;
const svgs = {
	'static/favicon.svg': icon({ corner: ROUNDED, size: 0.62 })
};
const pngs: Record<string, { px: number; svg: string }> = {
	'static/icons/icon-192.png': { px: 192, svg: icon({ corner: ROUNDED, size: 0.5 }) },
	'static/icons/icon-512.png': { px: 512, svg: icon({ corner: ROUNDED, size: 0.5 }) },
	// Maskable: full bleed, the ring inside the central 80% circle that every mask keeps.
	'static/icons/icon-maskable-512.png': { px: 512, svg: icon({ corner: 0, size: 0.44 }) },
	// iOS rounds the corners itself and fills transparency with black.
	'static/apple-touch-icon.png': { px: 180, svg: icon({ corner: 0, size: 0.5 }) }
};

for (const [path, svg] of Object.entries(svgs)) {
	await writeFile(resolve(root, path), svg);
}

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || undefined });
const page = await browser.newPage({ deviceScaleFactor: 1 });
for (const [path, { px, svg }] of Object.entries(pngs)) {
	await page.setViewportSize({ width: px, height: px });
	await page.setContent(
		`<style>*{margin:0}body{background:transparent}svg{display:block;width:${px}px;height:${px}px}</style>${svg}`
	);
	const out = resolve(root, path);
	await mkdir(dirname(out), { recursive: true });
	await page.screenshot({ path: out, omitBackground: true });
	console.log(`${path} ${px}px`);
}
await browser.close();
