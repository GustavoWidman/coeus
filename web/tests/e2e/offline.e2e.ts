import type { Page } from '@playwright/test';
import { expect, test } from './fleet.ts';

const row = (page: Page) => page.locator('[data-key="r-lab"]').first();
const tag = (page: Page) => page.locator('.live').first();

/** the fleet as the stream sent it, then kept on the device */
async function seen(page: Page) {
	await expect(row(page)).toBeVisible();
	await page.waitForTimeout(1000);
}

test('a 530 shows the kept fleet under the banner, and the stream takes over when a host is back', async ({
	open
}) => {
	const page = await open('/', 10);
	await seen(page);
	await page.route('**/api/**', (r) => r.fulfill({ status: 530, body: '' }));
	await page.reload();
	const banner = page.locator('.banner');
	await expect(banner).toContainText('Showing cached data');
	await expect(banner).toContainText('(530)');
	await expect(tag(page)).toContainText('cached');
	await expect(row(page).locator('.dot')).toHaveAttribute('data-s', 'stale');
	await page.unroute('**/api/**');
	await expect(banner).toHaveCount(0, { timeout: 20_000 });
	await expect(tag(page)).toContainText('live');
	await expect(row(page).locator('.dot')).not.toHaveAttribute('data-s', 'stale');
});

test("the offline Worker's 503 reads as every host down too", async ({ open }) => {
	const page = await open('/', 10);
	await seen(page);
	await page.route('**/api/**', (r) =>
		r.fulfill({
			status: 503,
			headers: { 'content-type': 'application/json', 'coeus-offline': '1' },
			body: '{"offline":true}'
		})
	);
	await page.reload();
	await expect(page.locator('.banner')).toContainText('(503)');
	await expect(tag(page)).toContainText('cached');
});

test('a 503 the Worker did not mark is one failed request, not every host down', async ({
	open
}) => {
	const page = await open('/', 10);
	await seen(page);
	await page.route('**/api/**', (r) => r.fulfill({ status: 503, body: '' }));
	await page.reload();
	await expect(row(page)).toBeVisible();
	await page.waitForTimeout(1000);
	await expect(page.locator('.banner')).toHaveCount(0);
	await expect(tag(page)).toContainText('connecting');
});
