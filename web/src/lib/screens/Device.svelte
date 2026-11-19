<!--
	This device: who it is signed in as and until when, and which host serves it. It loads only
	once opened.
-->
<script lang="ts">
	import { day } from '#lib/derive/format.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import Label from '#lib/ui/Label.svelte';
	import { noun, standalone } from '#lib/ui/platform.ts';

	const word = noun();
	const app = standalone();
</script>

{#if fleet.viewer}
	{@const v = fleet.viewer}
	<div class="ctr">
		<div class="lt">
			<div class="kicker">this {word}</div>
			<h1 class="mt">{v.device}</h1>
			<p class="sub">
				Signed in as {v.login.split('@')[0]}, {app ? 'from the Home Screen app' : 'in the browser'}.
			</p>
		</div>
		<Label a="Session" />
		<dl class="kv">
			<div>
				<dt>account</dt>
				<dd>{v.login}</dd>
			</div>
			<div>
				<dt>device</dt>
				<dd>{v.device} <span class="m">· {app ? 'Home Screen app' : 'browser'}</span></dd>
			</div>
			<div>
				<dt>until</dt>
				<dd>{day(v.expires_at)} <span class="m">· then sign in on the tailnet again</span></dd>
			</div>
			<div>
				<dt>served by</dt>
				<dd>
					{fleet.servedBy}
					<span class="m">
						· {fleet.link === 'live' ? 'stream' : fleet.link === 'down' ? 'cached' : fleet.link}
					</span>
				</dd>
			</div>
		</dl>
	</div>
{/if}
