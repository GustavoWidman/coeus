import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
	publicDir: 'static',
	// `coeus web --fixture instrument` serves a simulated fleet here
	server: {
		proxy: { '/api': 'http://127.0.0.1:7979', '/fixture': 'http://127.0.0.1:7979' }
	},
	build: {
		outDir: 'build',
		// Every name under this path carries a content hash, so the server caches it for a year.
		assetsDir: '_app/immutable',
		// Every browser the app supports preloads modules itself.
		modulePreload: { polyfill: false }
	},
	plugins: [svelte({ compilerOptions: { runes: true } })]
});
