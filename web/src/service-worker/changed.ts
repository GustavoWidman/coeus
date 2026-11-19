/**
 * Marks this device's subscription as changed, for the app to show the new one (see
 * `lib/push/changed.ts`, which reads and clears the mark). The service worker is a classic
 * script that cannot import the chunks the app shares, so it writes the mark itself, into the
 * database `lib/idb.ts` lays out: one object store named as the database, version 1.
 */
const NAME = 'coeus-push';

export function markChanged(): Promise<void> {
	return new Promise((ok, fail) => {
		const open = indexedDB.open(NAME, 1);
		open.onupgradeneeded = () => open.result.createObjectStore(NAME);
		open.onerror = () => fail(open.error);
		open.onsuccess = () => {
			const tx = open.result.transaction(NAME, 'readwrite');
			tx.objectStore(NAME).put(true, 'changed');
			tx.oncomplete = () => (open.result.close(), ok());
			tx.onerror = () => fail(tx.error);
		};
	});
}
