<script lang="ts">
	let { p, c = '', rule = false }: { p: number; c?: string; rule?: boolean } = $props();

	// the stylesheet draws a new gauge in with an animation of its transform; once that ends it
	// has to go, or the browser keeps every later move of the gauge off the compositor
	const drawn = (e: AnimationEvent) => {
		if (e.animationName === 'draw') (e.currentTarget as HTMLElement).style.animationName = 'none';
	};
</script>

<div class="{rule ? 'rule' : 'line'} {c}">
	<i style:--p={Math.max(0, Math.min(1, p)).toFixed(4)} onanimationend={drawn}></i>
</div>
