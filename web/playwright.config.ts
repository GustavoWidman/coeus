import { defineConfig } from '@playwright/test';

/**
 * End-to-end tests against the app as it ships: the production build served by
 * `coeus web --fixture instrument`, one server per worker. `nix develop -c e2e` builds both
 * and runs this.
 */
export default defineConfig({
	testDir: 'tests/e2e',
	testMatch: '**/*.e2e.ts',
	fullyParallel: true,
	retries: 0,
	timeout: 60_000,
	expect: { timeout: 7_000 },
	reporter: [['list']],
	use: {
		timezoneId: 'UTC',
		locale: 'en-US',
		trace: 'retain-on-failure',
		// a service worker would answer requests before a test's routes see them
		serviceWorkers: 'block'
	},
	projects: [
		{
			name: 'phone',
			use: {
				browserName: 'webkit',
				viewport: { width: 390, height: 844 },
				deviceScaleFactor: 3,
				isMobile: true,
				hasTouch: true
			}
		},
		{
			name: 'desktop',
			use: {
				browserName: 'chromium',
				viewport: { width: 1440, height: 900 },
				deviceScaleFactor: 1
			}
		}
	]
});
