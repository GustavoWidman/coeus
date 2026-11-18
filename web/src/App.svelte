<!--
	Signing in is a page of its own, and the fleet's screens load only past it, so a device that
	has to sign in first loads none of them.
-->
<script lang="ts">
	import { router } from '#lib/router.svelte.ts';

	const signin = $derived(router.nav.route.screen === 'signin');
</script>

{#if signin}
	{#await import('#lib/auth/SignIn.svelte') then m}<m.default />{/await}
{:else}
	{#await import('#lib/screens/Live.svelte') then m}<m.default />{/await}
{/if}
