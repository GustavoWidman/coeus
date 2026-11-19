import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';
import { Fixture, expect, test } from './fleet.ts';

/** a deploy's summary exactly as `coeus deploy` sends it, pinned by the sender's own test */
const SENT = readFileSync(new URL('../../../contract/push.json', import.meta.url), 'utf8').trim();
const ORDER = '9f3c0a1b2c3d4e5f';

// Playwright runs service workers in Chromium alone, and only Chromium's DevTools protocol
// delivers a push. The headless shell denies every notification, so these run in Chromium
// itself, headless.
test.skip(({ browserName }) => browserName !== 'chromium');
test.use({ serviceWorkers: 'allow', channel: 'chromium' });

test.describe('notifications', () => {
	let fixture: Fixture;
	test.beforeEach(async () => (fixture = await Fixture.start()));
	test.afterEach(() => fixture.stop());

	/** the app open on the fleet, its service worker in charge, allowed to notify */
	async function installed(page: Page) {
		await page.context().grantPermissions(['notifications'], { origin: fixture.base });
		await page.goto(fixture.base + '/');
		await page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined));
		await expect(page.locator('[data-key="r-lab"]').first()).toBeVisible();
	}

	/** delivers `data` to the service worker as the push service would */
	async function deliver(page: Page, data: string) {
		const cdp = await page.context().newCDPSession(page);
		const registration = new Promise<string>((ok) =>
			cdp.on('ServiceWorker.workerRegistrationUpdated', ({ registrations }) => {
				const mine = registrations.find((r) => r.scopeURL === `${fixture.base}/` && !r.isDeleted);
				if (mine) ok(mine.registrationId);
			})
		);
		await cdp.send('ServiceWorker.enable');
		await cdp.send('ServiceWorker.deliverPushMessage', {
			origin: fixture.base,
			registrationId: await registration,
			data
		});
	}

	const shown = (page: Page) =>
		page.evaluate(async () => {
			const reg = await navigator.serviceWorker.ready;
			return (await reg.getNotifications()).map((n) => ({
				title: n.title,
				body: n.body,
				tag: n.tag,
				data: n.data
			}));
		});

	/** what the fleet sends, its links on the fixture's origin rather than the fleet's */
	const sent = () => SENT.replaceAll('https://coeus.example.com', fixture.base);

	test("a deploy's summary shows as the fleet sent it, its link a path of the app", async ({
		page
	}) => {
		await installed(page);
		await deliver(page, sent());
		await expect
			.poll(() => shown(page))
			.toEqual([
				{
					title: 'Deployed caf4e2b · 2 of 3',
					body: 'chore(flake): bump nixpkgs\n2 switched in 6m40s\npending: laptop-mac (asleep)',
					tag: 'deploy/switch/caf4e2b9a1d6',
					data: { navigate: `/deployments/${ORDER}` }
				}
			]);
	});

	test('a push it cannot read still shows a notification, so the permission stays', async ({
		page
	}) => {
		await installed(page);
		await deliver(page, 'not a declarative message');
		await expect
			.poll(() => shown(page))
			.toEqual([{ title: 'coeus', body: '', tag: '', data: { navigate: '/' } }]);
	});

	test('a tap takes the open window to the screen the notification names', async ({ page }) => {
		await installed(page);
		await deliver(page, sent());
		await expect.poll(() => shown(page)).toHaveLength(1);
		const [worker] = page.context().serviceWorkers();
		// a tap is the browser's to make, so the worker hears one it did not make; the window
		// is told where to go before the handler asks the event to wait
		await worker.evaluate(async () => {
			type Click = new (type: string, init: { notification: Notification }) => Event;
			const sw = self as unknown as EventTarget & { registration: ServiceWorkerRegistration };
			const [notification] = await sw.registration.getNotifications();
			const NotificationEvent = (globalThis as unknown as { NotificationEvent: Click })
				.NotificationEvent;
			try {
				sw.dispatchEvent(new NotificationEvent('notificationclick', { notification }));
			} catch {}
		});
		await expect(page).toHaveURL(`${fixture.base}/deployments/${ORDER}`);
		await expect(page.locator('.top .crumbs b')).toHaveText('deployment');
		// the simulated fleet never ran this one, as a deployment past the recent window
		await expect(page.locator('.pane.mid')).toContainText('No deployment by this name');
		expect(await shown(page)).toHaveLength(0);
	});
});
