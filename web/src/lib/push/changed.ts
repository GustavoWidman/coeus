/**
 * Whether this device's subscription changed since the subscription list last held it: the
 * browser replaced it (the service worker's `pushsubscriptionchange`), or the app subscribed
 * again under a new key. The service worker writes the same mark into the same store
 * (`service-worker/changed.ts`).
 */
import { store } from '../idb.ts';

const db = store('coeus-push');
const KEY = 'changed';

export const changed = {
	mark: () => db.put(KEY, true),
	clear: () => db.put(KEY, false),
	async read(): Promise<boolean> {
		try {
			return (await db.get<boolean>(KEY)) === true;
		} catch {
			return false;
		}
	}
};
