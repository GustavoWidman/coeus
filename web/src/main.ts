import { mount } from 'svelte';
import '#lib/ui/fonts.css';
import '#lib/ui/app.css';
import App from './App.svelte';

mount(App, { target: document.getElementById('app')! });
