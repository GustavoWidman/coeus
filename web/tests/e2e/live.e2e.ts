import type { Locator } from '@playwright/test';
import { expect, test } from './fleet.ts';

/** the text a row says once its rolling digits settle */
const said = (row: Locator) =>
	row.evaluate((e) => {
		const copy = e.cloneNode(true) as Element;
		for (const old of copy.querySelectorAll('.dg > .o')) old.remove();
		return copy.textContent ?? '';
	});
const builds = (text: string) => Number(/building · ([\d,]+)\//.exec(text)?.[1].replace(/,/g, ''));

test('a build counts up in place, its changed digits rolling', async ({ open }) => {
	const page = await open('/', 10, 1);
	const status = page.locator('[data-key="r-lab"] .row-s');
	await expect(status).toContainText('building · ');
	const first = builds(await said(status));
	expect(first).toBeGreaterThan(0);
	await expect
		.poll(async () => builds(await said(status)), { timeout: 5000 })
		.toBeGreaterThan(first);
	await expect(page.locator('[data-key="r-lab"] .row-s .dg').first()).toBeAttached();
	const rule = page.locator('[data-key="r-lab"] .rule > i');
	const p = Number(await rule.evaluate((e) => (e as HTMLElement).style.getPropertyValue('--p')));
	expect(p).toBeGreaterThan(0.2);
});

test('a finished job slides into the history and the next one starts', async ({ open }) => {
	// #61 verifies until 35.55 s, then #62, queued behind it, starts at 37.5 s
	const page = await open('/machines/lab', 34.5, 1);
	const history = page.locator('[data-key="hist"]');
	await expect(page.locator('[data-key="run-61"]')).toBeVisible();
	await expect(history.locator('[data-key="j-61"]')).toBeVisible({ timeout: 5000 });
	await expect(history.locator('li').first()).toHaveAttribute('data-key', 'j-61');
	await expect(history.locator('[data-key="j-61"] .js')).toContainText('succeeded');
	await expect(page.locator('[data-key="run-61"]')).toHaveCount(0);
	await expect(page.locator('[data-key="run-62"] .lbl')).toContainText(/Starting|Running/);
	await expect(page.locator('.lt .status')).toContainText(/starting #62|preparing|fetching/);
});

test('a machine that wakes catches up', async ({ open }) => {
	// laptop-mac wakes at 67 s and catches up with #37
	const page = await open('/', 65.5, 1);
	const row = page.locator('[data-key="r-laptop-mac"]');
	await expect(row.locator('.row-s')).toContainText('asleep');
	await expect(row.locator('.row-s')).toContainText(/starting #37|catching up/, { timeout: 5000 });
	await expect(row.locator('.dot')).not.toHaveAttribute('data-s', 'asleep');
});

test.describe('with reduced motion', () => {
	test.use({ reducedMotion: 'reduce' });

	test('updates start no animation', async ({ open }) => {
		const page = await open('/machines/lab', 34.5, 1);
		await expect(page.locator('[data-key="hist"] [data-key="j-61"]')).toBeVisible({
			timeout: 5000
		});
		const lasting = await page.evaluate(() =>
			document
				.getAnimations()
				.filter(
					(a) => Number(a.effect?.getComputedTiming().duration) > 1 && a.playState === 'running'
				)
				.map((a) => (a as CSSAnimation).animationName ?? a.constructor.name)
		);
		expect(lasting).toEqual([]);
		await expect(page.locator('.dg')).toHaveCount(0);
	});
});
