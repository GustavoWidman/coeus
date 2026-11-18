const PERIOD = 15000;
const line = document.querySelector('.line > i');
const ago = document.getElementById('ago');
const still = matchMedia('(prefers-reduced-motion: reduce)');
let checked = Date.now();
let timer = 0;
let probing = false;

// Asks for this same page again, and reloads once the answer no longer comes from the offline
// Worker. A hidden tab waits, and checks as soon as it is shown.
async function probe() {
	if (probing || document.hidden) return;
	probing = true;
	clearTimeout(timer);
	try {
		const res = await fetch(location.href, { method: 'HEAD', cache: 'no-store' });
		if (!res.headers.has('coeus-offline')) return location.reload();
	} catch {}
	probing = false;
	checked = Date.now();
	tell();
	wait();
}

function wait() {
	timer = setTimeout(probe, PERIOD);
	if (!still.matches) {
		line.animate([{ transform: 'scaleX(1)' }, { transform: 'scaleX(0)' }], PERIOD);
	}
}

function tell() {
	const s = Math.round((Date.now() - checked) / 1000);
	ago.textContent = s < 2 ? 'no answer · checked just now' : `no answer · checked ${s}s ago`;
}

document.getElementById('copy').addEventListener('click', async (e) => {
	const button = e.currentTarget;
	try {
		await navigator.clipboard.writeText(document.getElementById('cmd').textContent);
		button.textContent = 'copied';
		setTimeout(() => (button.textContent = 'copy'), 1500);
	} catch {}
});
document.addEventListener('visibilitychange', probe);
addEventListener('online', probe);
setInterval(tell, 1000);
tell();
wait();
