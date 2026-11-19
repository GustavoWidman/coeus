import { expect, test } from './fleet.ts';

test.describe('this device', () => {
	test("the fleet's foot opens it, and back returns to the fleet", async ({ open, isMobile }) => {
		test.skip(!isMobile, 'the desktop reaches it from its header');
		const page = await open('/', 100);
		await page.locator('.foot').click();
		await expect(page).toHaveURL(/\/device$/);
		await expect(page.locator('.nav-t').last()).toHaveText('This device');
		await expect(page.locator('.kv').last()).toContainText('gustavo@github');
		await page.locator('.back').last().click();
		await expect(page).toHaveURL(/\/$/);
	});

	test("the header's account opens it in the middle", async ({ open, isMobile }) => {
		test.skip(isMobile, 'a phone reaches it from the fleet');
		const page = await open('/', 100);
		await page.locator('.top .me').click();
		await expect(page).toHaveURL(/\/device$/);
		await expect(page.locator('.top .crumbs b')).toHaveText('device');
		await expect(page.locator('.pane.mid .kv')).toContainText('gustavo@github');
	});

	test('a deep link opens it over the fleet', async ({ open, isMobile }) => {
		const page = await open('/device', 100);
		await expect(page.locator('h1.mt')).toHaveText('iphone');
		if (isMobile) {
			await page.locator('.back').last().click();
			await expect(page).toHaveURL(/\/$/);
		}
	});
});
