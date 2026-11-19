/**
 * The browser's side of notifications, as a state needs it: whether the page has push at all,
 * what it was granted, the subscription it holds, and what subscribing answers. A headless
 * browser has no push service to subscribe with, so the matrix answers for one. The e2e tests
 * drive the real service worker.
 *
 * Playwright sends `stubPush` as source, so it uses nothing from outside its body.
 */

export interface PushStub {
	/** the page has `PushManager`; an iPhone's Safari tab does not */
	supported?: boolean;
	/** the page runs on an iPhone */
	ios?: boolean;
	/** as a Home Screen app */
	standalone?: boolean;
	permission: NotificationPermission;
	/** the subscription the browser holds, and the key it was made with */
	subscription: { endpoint: string; key: string } | null;
	/** what subscribing makes, or `hold` for a prompt that stays up */
	subscribe: { endpoint: string } | 'hold';
}

export function stubPush(stub: PushStub) {
	const P256DH =
		'BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QTpQtUbVlUls0VJXg7A8u-Ts1XbjhazAkj7I99e8QcYP7DkM';
	const AUTH = 'tBHItJI5svbpez7KI4CCXg';
	const raw = (b64: string) =>
		Uint8Array.from(atob(b64.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0)).buffer;
	const text = (key: BufferSource | string) => {
		if (typeof key === 'string') return key;
		const bytes = new Uint8Array(key instanceof ArrayBuffer ? key : key.buffer);
		return btoa(String.fromCharCode(...bytes))
			.replace(/\+/g, '-')
			.replace(/\//g, '_')
			.replace(/=+$/, '');
	};

	let permission = stub.permission;
	let current: object | null = null;
	const make = (endpoint: string, key: string) => {
		const sub = {
			endpoint,
			expirationTime: null,
			options: { userVisibleOnly: true, applicationServerKey: raw(key) },
			getKey: () => null,
			toJSON: () => ({ endpoint, expirationTime: null, keys: { p256dh: P256DH, auth: AUTH } }),
			unsubscribe: async () => {
				if (current === sub) current = null;
				return true;
			}
		};
		return (current = sub);
	};
	if (stub.subscription) make(stub.subscription.endpoint, stub.subscription.key);

	const pushManager = {
		getSubscription: async () => current,
		permissionState: async () => (permission === 'default' ? 'prompt' : permission),
		subscribe: (options: PushSubscriptionOptionsInit) => {
			const answer = stub.subscribe;
			if (answer === 'hold') return new Promise(() => {});
			return Promise.resolve().then(() => {
				permission = 'granted';
				return make(answer.endpoint, text(options.applicationServerKey!));
			});
		}
	};
	const registration = { pushManager, update: async () => {} };
	const container = {
		controller: null,
		ready: Promise.resolve(registration),
		register: async () => registration,
		getRegistration: async () => registration,
		addEventListener() {},
		removeEventListener() {}
	};
	Object.defineProperty(Navigator.prototype, 'serviceWorker', { get: () => container });
	if (stub.supported === false) delete (window as { PushManager?: unknown }).PushManager;
	else (window as { PushManager?: unknown }).PushManager ??= function PushManager() {};
	if (!('Notification' in window)) (window as { Notification?: unknown }).Notification = {};
	Object.defineProperty(Notification, 'permission', { get: () => permission });
	if (stub.ios)
		Object.defineProperty(Navigator.prototype, 'userAgent', {
			get: () =>
				'Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1'
		});
	if (stub.standalone)
		Object.defineProperty(Navigator.prototype, 'standalone', { get: () => true });
}
