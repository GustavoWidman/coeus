<!--
	The packages a job changed, read once a row opens and kept, since a written diff never
	changes. This and the reading load only when a row first opens.
-->
<script module lang="ts">
	import { device } from '#lib/auth/device.ts';
	import { parseDiff } from '#lib/wire/parse-log.ts';
	import type { DiffView } from '#lib/wire/types.ts';

	export type Read = { diff: DiffView } | { error: string };

	const read: Map<string, DiffView> = new Map();

	export async function load(host: string, job: number): Promise<Read> {
		const key = `${host}/${job}`;
		const known = read.get(key);
		if (known) return { diff: known };
		try {
			const path = `/api/machines/${encodeURIComponent(host)}/jobs/${job}/diff`;
			const res = await fetch(path, { headers: await device.headers('GET', path) });
			// the server answers 404 for a job that wrote a diff without changes, or none at all
			const diff =
				res.status === 404 ? { changes: [] } : res.ok ? parseDiff(await res.text()) : null;
			if (!diff) return { error: `the server answered ${res.status}` };
			read.set(key, diff);
			return { diff };
		} catch (e) {
			return { error: e instanceof Error ? e.message : String(e) };
		}
	}
</script>

<script lang="ts">
	import { delta } from '#lib/derive/format.ts';

	let { diff }: { diff: DiffView } = $props();

	const most = $derived(Math.max(1, ...diff.changes.map((c) => Math.abs(c.size_delta))));
	const total = $derived(diff.changes.reduce((a, c) => a + c.size_delta, 0));
</script>

{#each diff.changes as c (c.name)}
	<div class="drow">
		<span class="pkg">{c.name}</span><span class="ver">
			{#if c.before.length}{c.before.join(', ')}{:else}<b>new</b>{/if} →
			{#if c.after.length}<b>{c.after.join(', ')}</b>{:else}removed{/if}
		</span><span class="gauge">
			<span class="g" class:neg={c.size_delta < 0}>
				<i style:--g={(c.size_delta / most).toFixed(3)}></i>
			</span><em>{delta(c.size_delta)}</em>
		</span>
	</div>
{/each}
<div class="dsum">
	{#if diff.changes.length}
		<span>
			{diff.changes.length}
			{diff.changes.length === 1 ? 'package' : 'packages'} changed
		</span><span>closure {delta(total)} MiB</span>
	{:else}
		<span>No packages changed.</span>
	{/if}
</div>
