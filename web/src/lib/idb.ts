/** One IndexedDB database holding one object store, read and written a key at a time. */
export function store(name: string) {
	let db: Promise<IDBDatabase> | null = null;
	const open = () =>
		(db ??= new Promise((ok, fail) => {
			const r = indexedDB.open(name, 1);
			r.onupgradeneeded = () => r.result.createObjectStore(name);
			r.onsuccess = () => ok(r.result);
			r.onerror = () => fail(r.error);
		}));
	const run = async <T>(mode: IDBTransactionMode, op: (s: IDBObjectStore) => IDBRequest<T>) => {
		const r = op((await open()).transaction(name, mode).objectStore(name));
		return new Promise<T>((ok, fail) => {
			r.onsuccess = () => ok(r.result);
			r.onerror = () => fail(r.error);
		});
	};
	return {
		get: <T>(key: string) => run('readonly', (s) => s.get(key)) as Promise<T | undefined>,
		put: (key: string, value: unknown) => run('readwrite', (s) => s.put(value, key))
	};
}
