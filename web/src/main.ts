import { mount } from 'svelte';
import '#lib/ui/fonts.css';
import '#lib/ui/app.css';
import '#lib/auth/device.ts';
import App from './App.svelte';

mount(App, { target: document.getElementById('app')! });
