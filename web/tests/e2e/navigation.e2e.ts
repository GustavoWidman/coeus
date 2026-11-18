import { expect, standalone, test } from './fleet.ts';

test.describe('on a phone', () => {
	test.skip(({ isMobile }) => !isMobile);

	const top = (page: import('@playwright/test').Page) => page.locator('.stage > .screen').last();

	test('a machine pushes over the fleet and back returns to it', async ({ open }) => {
		const page = await open('/', 10);
		await page.locator('[data-key="r-lab"] a').click();
		await expect(page).toHaveURL(/\/machines\/lab$/);
		await expect(top(page).locator('h1')).toHaveText('lab');
		await expect(page.locator('.screen')).toHaveCount(2);
		await top(page).locator('.back').click();
		await expect(page).toHaveURL(/\/$/);
		await expect(page.locator('.screen')).toHaveCount(1);
		await expect(top(page).locator('h1')).toHaveText('Fleet');

		await page.locator('[data-key="r-oracle-1"] a').click();
		await expect(top(page).locator('h1')).toHaveText('oracle-1');
		await page.goBack();
		await expect(page.locator('.screen')).toHaveCount(1);
		await page.goForward();
		await expect(top(page).locator('h1')).toHaveText('oracle-1');
	});

	test('a link straight to a machine has the fleet under it', async ({ open }) => {
		const page = await open('/machines/altar', 10);
		await expect(top(page).locator('h1')).toHaveText('altar');
		await expect(page.locator('.screen')).toHaveCount(2);
		await top(page).locator('.back').click();
		await expect(page).toHaveURL(/\/$/);
		await expect(top(page).locator('h1')).toHaveText('Fleet');
	});

	test('the deployment opens from the fleet', async ({ open }) => {
		const page = await open('/', 10);
		await page.locator('.dep').click();
		await expect(page).toHaveURL(/\/deployments\/2d2eef4459f7ebbf$/);
		await expect(top(page).locator('h1')).toHaveText('main@2cbe6da');
	});

	test.describe('as a Home Screen app', () => {
		test.beforeEach(({ page }) => standalone(page));

		const swipe = async (page: import('@playwright/test').Page, to: number) => {
			await page.mouse.move(4, 400);
			await page.mouse.down();
			for (let x = 20; x <= to; x += 20) await page.mouse.move(x, 400);
			await page.mouse.up();
		};

		test('a swipe from the left edge goes back', async ({ open }) => {
			const page = await open('/', 10);
			await page.locator('[data-key="r-lab"] a').click();
			await expect(top(page).locator('h1')).toHaveText('lab');
			await page.waitForTimeout(400);
			await swipe(page, 300);
			await expect(page).toHaveURL(/\/$/);
			await expect(page.locator('.screen')).toHaveCount(1);
			await expect(top(page).locator('h1')).toHaveText('Fleet');
			// the gesture already moved the screens, so the history step that follows moves nothing
			await page.waitForTimeout(400);
			await expect(page.locator('.screen')).toHaveCount(1);
		});

		test('a short swipe springs back', async ({ open }) => {
			const page = await open('/machines/lab', 10);
			await expect(top(page).locator('h1')).toHaveText('lab');
			await page.mouse.move(4, 400);
			await page.mouse.down();
			await page.mouse.move(40, 400, { steps: 4 });
			await page.waitForTimeout(300);
			await page.mouse.move(60, 400, { steps: 4 });
			await page.mouse.up();
			await page.waitForTimeout(400);
			await expect(page).toHaveURL(/\/machines\/lab$/);
			await expect(page.locator('.screen')).toHaveCount(2);
			expect(await top(page).evaluate((e) => getComputedStyle(e).transform)).toBe('none');
		});
	});

	test('in a browser tab the left edge is left to the browser', async ({ open }) => {
		const page = await open('/machines/lab', 10);
		await page.mouse.move(4, 400);
		await page.mouse.down();
		await page.mouse.move(300, 400, { steps: 10 });
		await page.mouse.up();
		await page.waitForTimeout(400);
		await expect(page).toHaveURL(/\/machines\/lab$/);
	});
});

test.describe('on a desktop', () => {
	test.skip(({ isMobile }) => isMobile);

	test('the middle shows the newest deployment that reached more than one machine', async ({
		open
	}) => {
		// at 10 s, a newer order of a snapshot to lab alone waits behind lab's build
		const page = await open('/', 10);
		await expect(page.locator('.pane.mid h1')).toHaveText('main@2cbe6da');
		await expect(page.locator('.pane.left .dep .src')).toHaveText('main@2cbe6da');
		await expect(page.locator('.top .crumbs b')).toHaveText('deployment');
	});

	test("a one-machine order shows in that machine's queue and history", async ({ open }) => {
		const page = await open('/machines/lab', 10);
		await expect(page.locator('[data-key="q"] [data-key="j-62"]')).toContainText(
			'snapshot 3f9a1c0'
		);
		const later = await open('/machines/lab', 58);
		await expect(later.locator('[data-key="hist"] [data-key="j-62"]')).toContainText(
			'snapshot 3f9a1c0'
		);
		await expect(later.locator('.pane.left .dep .src')).toHaveText('main@2cbe6da');
	});

	test('picking a machine shows it in the middle and marks its row', async ({ open }) => {
		const page = await open('/', 10);
		await page.locator('[data-key="r-desktop-nixos"] a').click();
		await expect(page).toHaveURL(/\/machines\/desktop-nixos$/);
		await expect(page.locator('.pane.mid h1')).toHaveText('desktop-nixos');
		await expect(page.locator('[data-key="r-desktop-nixos"] .row')).toHaveClass(/sel/);
		await expect(page.locator('.top .crumbs b')).toHaveText('desktop-nixos');
		await page.goBack();
		await expect(page.locator('.pane.mid h1')).toHaveText('main@2cbe6da');
		await expect(page.locator('.row.sel')).toHaveCount(0);
	});
});
