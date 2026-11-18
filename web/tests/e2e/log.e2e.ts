import type { Page } from '@playwright/test';
import { expect, test } from './fleet.ts';

const LOG = '/machines/lab/jobs/61';

/** how far the log is from its end, in pixels */
const fromEnd = (page: Page) =>
	page.locator('.logbody').evaluate((b) => b.scrollHeight - b.scrollTop - b.clientHeight);

test.describe('the log sheet on a phone', () => {
	test.skip(({ isMobile }) => !isMobile);

	/** drags the sheet's header down by `dy` in `steps` moves, `pause` ms apart */
	async function drag(page: Page, dy: number, steps: number, pause: number) {
		const box = (await page.locator('.sheet .logh').boundingBox())!;
		const x = box.x + 60;
		const y = box.y + 24;
		await page.mouse.move(x, y);
		await page.mouse.down();
		for (let i = 1; i <= steps; i++) {
			await page.mouse.move(x, y + (dy * i) / steps);
			if (pause) await page.waitForTimeout(pause);
		}
		await page.mouse.up();
	}

	test('rises over the machine it belongs to', async ({ open }) => {
		const page = await open('/machines/lab', 10);
		await page.locator('.loglink').click();
		await expect(page).toHaveURL(new RegExp(`${LOG}$`));
		await expect(page.locator('.sheet .logh .t')).toContainText('#61 switch');
		await expect(page.locator('.sheet .ln').first()).toBeVisible();
		await page.locator('.done-btn').click();
		await expect(page).toHaveURL(/\/machines\/lab$/);
		await expect(page.locator('.sheet-wrap')).toHaveCount(0);
		await expect(page.locator('.stage > .screen').last().locator('h1')).toHaveText('lab');
	});

	test('dragged down past its threshold, it goes', async ({ open }) => {
		const page = await open(LOG, 10);
		await expect(page.locator('.sheet .ln').first()).toBeVisible();
		await page.waitForTimeout(400);
		await drag(page, 220, 6, 0);
		await expect(page).toHaveURL(/\/machines\/lab$/);
		await expect(page.locator('.sheet-wrap')).toHaveCount(0);
		await expect(page.locator('#app')).not.toHaveClass(/sheeted/);
	});

	test('dragged slowly a little way, it settles back', async ({ open }) => {
		const page = await open(LOG, 10);
		await expect(page.locator('.sheet .ln').first()).toBeVisible();
		await page.waitForTimeout(400);
		await drag(page, 70, 7, 40);
		await page.waitForTimeout(400);
		await expect(page).toHaveURL(new RegExp(`${LOG}$`));
		const top = await page.locator('.sheet').evaluate((s) => s.getBoundingClientRect().top);
		const wrap = await page.locator('.sheet-wrap').evaluate((s) => s.getBoundingClientRect().top);
		expect(top - wrap).toBeLessThan(12);
	});
});

test('scrolled up, new lines are counted until the pill takes the log to its end', async ({
	open,
	fleet
}) => {
	const page = await open(LOG, 10);
	await expect(page.locator('.ln').first()).toBeVisible();
	await page.locator('.logbody').evaluate((b) => b.scrollTo({ top: 0 }));
	await expect(page.locator('.jump')).not.toHaveClass(/\bon\b/);
	await fleet.seek(10, 1);
	await expect(page.locator('.jump')).toHaveClass(/\bon\b/, { timeout: 10_000 });
	expect(Number(await page.locator('.jump .jn').textContent())).toBeGreaterThan(0);
	await fleet.seek(13);
	await page.locator('.jump').click();
	await expect.poll(() => fromEnd(page)).toBeLessThan(48);
	await expect(page.locator('.jump')).not.toHaveClass(/\bon\b/);
});

test('a deployment row opens into its diff, read only then', async ({ open }) => {
	const page = await open('/deployments/2d2eef4459f7ebbf', 10);
	const asked: string[] = [];
	page.on('request', (r) => {
		if (/\/diff$|DiffRows/.test(r.url())) asked.push(r.url());
	});
	const row = page.locator('.vrow', { hasText: 'oracle-1' }).last();
	await expect(row).toBeVisible();
	await page.waitForTimeout(300);
	expect(asked).toEqual([]);
	await row.click();
	const diff = page.locator('.diff').last();
	await expect(diff.locator('.drow')).toHaveCount(5);
	await expect(diff.locator('.dsum')).toContainText('5 packages changed');
	await expect(diff.locator('.dsum')).toContainText('closure +4.0 MiB');
	expect(asked.some((u) => u.endsWith('/machines/oracle-1/jobs/24/diff'))).toBe(true);
	await row.click();
	await expect(page.locator('.diff')).toHaveCount(0);
});

test('a log of twenty thousand lines draws only the lines on screen', async ({
	open,
	page,
	browserName
}) => {
	const lines = Array.from({ length: 20_000 }, (_, i) => ({
		at: null,
		kind: i % 4 === 3 ? 'fetch' : 'build',
		text: `building '/nix/store/${String(i).padStart(8, '0')}-package-${i}.drv'`,
		prefix: null
	}));
	const started = {
		kind: 'started',
		job: {
			id: 61,
			order: '2d2eef4459f7ebbf',
			source: { kind: 'git', rev: '2cbe6da7e56b76230b636eccac43a797985e81e1', git_ref: 'main' },
			action: 'switch',
			origin: { kind: 'follow' },
			state: { kind: 'finished', outcome: { kind: 'succeeded' } },
			ordered_at: 0,
			started_at: 0,
			finished_at: 1,
			phase_started_at: null,
			toplevel: null,
			attempt: 1,
			changes: null
		}
	};
	const body = [
		started,
		{ kind: 'lines', lines, end: 1 },
		{ kind: 'ended', outcome: { kind: 'succeeded' } }
	]
		.map((e, i) => `id: ${i}\nevent: ${e.kind}\ndata: ${JSON.stringify(e)}\n\n`)
		.join('');
	await page.route('**/api/machines/lab/jobs/61/log', (r) =>
		r.fulfill({ contentType: 'text/event-stream', body })
	);
	// the most lines the page ever holds at once, not only once it settles
	await page.addInitScript(() => {
		const w = window as unknown as { most: number };
		w.most = 0;
		const lines = document.getElementsByClassName('ln');
		new MutationObserver(() => (w.most = Math.max(w.most, lines.length))).observe(document, {
			childList: true,
			subtree: true
		});
	});
	await open(LOG, 100);
	const last = page.locator('.ln .no', { hasText: /^20000$/ });
	await expect(last).toBeVisible();
	expect(await page.evaluate(() => (window as unknown as { most: number }).most)).toBeLessThan(400);

	const nodes = () =>
		page.evaluate(() => {
			let n = 0;
			const walk = document.createTreeWalker(document, NodeFilter.SHOW_ALL);
			while (walk.nextNode()) n++;
			return n;
		});
	expect(await nodes()).toBeLessThan(2_500);

	await page.locator('.logbody').evaluate((b) => b.scrollTo({ top: 0 }));
	await expect(page.locator('.ln .no', { hasText: /^1$/ })).toBeVisible();
	await page.locator('.logbody').evaluate((b) => b.scrollTo({ top: b.scrollHeight / 2 }));
	await expect(page.locator('.ln .no', { hasText: /^1\d{4}$/ }).first()).toBeVisible();
	expect(await nodes()).toBeLessThan(2_500);

	if (browserName === 'chromium') {
		const cdp = await page.context().newCDPSession(page);
		await cdp.send('HeapProfiler.collectGarbage');
		await cdp.send('Performance.enable');
		const { metrics } = await cdp.send('Performance.getMetrics');
		const heap = metrics.find((m) => m.name === 'JSHeapUsedSize')!.value;
		expect(heap / 1_048_576).toBeLessThan(12);
	}
});
