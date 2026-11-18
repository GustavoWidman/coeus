<script lang="ts">
	import type { Dot } from '#lib/derive/status.ts';
	import { pulse } from '#lib/motion/pulse.ts';

	/** `beat` rings the dot too, without changing what it shows */
	let { s, beat = 0 }: { s: Dot; beat?: number } = $props();
</script>

<i class="dot" data-s={s} {@attach pulse(`${s}.${beat}`)}><b></b></i>
