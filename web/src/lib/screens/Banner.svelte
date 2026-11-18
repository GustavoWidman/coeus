<!--
	Every host is down: what the screen shows is the fleet as it last was, if this device has it,
	and the app asks again every 15 s as the hairline drains.
-->
<script lang="ts">
	import { clk, hms } from '#lib/derive/format.ts';
	import { enter, ghost } from '#lib/motion/flip.ts';
	import { fleet } from '#lib/stream/fleet.svelte.ts';
	import { now } from '#lib/time/now.svelte.ts';
	import Dot from '#lib/ui/Dot.svelte';
	import Tick from '#lib/ui/Tick.svelte';
</script>

<div class="banner" data-key="banner" in:enter out:ghost>
	<Dot s="stale" />
	{#if fleet.ready && fleet.asOf !== null}
		<b>Showing cached data</b><span class="age">
			<Tick still v={clk(now.ms - fleet.asOf)} /> old
		</span>
		<p>
			No coeusd answered through the tunnel ({fleet.down}). This is the fleet as of {hms(
				fleet.asOf
			)}.
		</p>
	{:else}
		<b>No coeus host answered</b><span class="age"></span>
		<p>
			No coeusd answered through the tunnel ({fleet.down}), and this device keeps no fleet from
			before.
		</p>
	{/if}
	<div class="retry">
		<span>retrying every 15s</span>
		<div class="line"><i style="--p:1"></i></div>
	</div>
</div>
