import type { Page } from '@playwright/test';
import { expect, test } from './fleet.ts';

// a fleet that asks its readers to sign in, as a serving host does; the fixture signs in
// whoever asks on its own address, which stands in for the tailnet listener
test.use({ signin: true });

interface Stored {
	keys: CryptoKeyPair;
	session: { token: string; iat: number } | null;
}

/** what the device keeps in IndexedDB, or rewrites it with `iat` moved back */
const device = (page: Page, rewind = 0) =>
	page.evaluate(async (rewind) => {
		const db = await new Promise<IDBDatabase>((ok) => {
			const r = indexedDB.open('coeus-device', 1);
			r.onsuccess = () => ok(r.result);
		});
		const store = (mode: IDBTransactionMode) =>
			db.transaction('coeus-device', mode).objectStore('coeus-device');
		const stored = await new Promise<Stored>((ok) => {
			const r = store('readonly').get('this');
			r.onsuccess = () => ok(r.result);
		});
		if (rewind && stored.session) {
			stored.session.iat -= rewind;
			await new Promise((ok) => (store('readwrite').put(stored, 'this').onsuccess = ok));
		}
		return {
			extractable: stored.keys.privateKey.extractable,
			token: stored.session?.token ?? null,
			iat: stored.session?.iat ?? null
		};
	}, rewind);

const rows = (page: Page) => page.locator('[data-key="r-lab"]').first();

async function signIn(page: Page, base: string) {
	await page.goto(base + '/');
	await expect(page).toHaveURL(/\/signin$/);
	await expect(page.locator('.auth h1')).toHaveText('Signed in on the tailnet');
}

test('a device without a session signs itself in on the tailnet, then opens the fleet', async ({
	page,
	fleet
}) => {
	await signIn(page, fleet.base);
	await expect(page.locator('.kv')).toContainText('gustavo');
	await expect(page.locator('.kv')).toContainText('7 days');
	const stored = await device(page);
	expect(stored.extractable).toBe(false);
	expect(stored.token).toMatch(/^v1\./);
	await page.locator('.btn').click();
	await expect(page).toHaveURL(/\/$/);
	await expect(rows(page)).toBeVisible();
	await expect(page.locator('.live').first()).toContainText('live');
});

test('a session the server stops taking ends on the sign-in page', async ({ page, fleet }) => {
	// signed in before the fleet's screens first load, so their stream starts with the session
	await page.goto(fleet.base + '/signin');
	await expect(page.locator('.auth h1')).toHaveText('Signed in on the tailnet');
	await page.route('**/auth/ping', (r) => r.abort('connectionrefused'));
	await page.route('**/api/stream', (r) => r.fulfill({ status: 401, body: '' }));
	await page.locator('.btn').click();
	await expect(page).toHaveURL(/\/signin$/);
	await expect(page.locator('.auth h1')).toHaveText('Your session ended');
	await expect(page.locator('.probe')).toContainText('not on the tailnet · checked');
});

test('a device the tailnet policy keeps out is told so, and asks again only when told to', async ({
	page,
	fleet
}) => {
	let asked = 0;
	await page.route('**/auth/session', (r) => {
		asked++;
		return r.fulfill({ status: 403, body: 'the tailnet policy grants no access' });
	});
	await page.goto(fleet.base + '/signin');
	await expect(page.locator('.auth h1')).toHaveText('This device may not sign in');
	await expect(page.locator('.probe')).toContainText('the tailnet policy grants no access');
	await page.waitForTimeout(3500);
	expect(asked).toBe(1);
	await page.locator('.btn').click();
	await expect.poll(() => asked).toBe(2);
});
