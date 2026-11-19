import type { MachineState, Ms, OriginView, SourceView } from '#lib/wire/types.ts';

const S = 1e3;
const M = 6e4;
const H = 36e5;
const D = 864e5;
const MiB = 1_048_576;

export const num = (x: number) => Math.round(x).toLocaleString('en-US');
export const mib = (bytes: number) => (bytes / MiB).toFixed(1);
/** a change in size, in MiB, with its sign */
export const delta = (bytes: number) => {
	const d = bytes / MiB;
	return (d > 0 ? '+' : d < 0 ? '−' : '±') + Math.abs(d).toFixed(1);
};
const pad = (x: number) => String(x).padStart(2, '0');

export function ago(t: Ms, now: Ms) {
	const d = Math.max(0, now - t);
	if (d < 5e3) return 'now';
	if (d < M) return Math.floor(d / S) + 's ago';
	if (d < H) return Math.floor(d / M) + 'm ago';
	if (d < D) return Math.floor(d / H) + 'h ago';
	return Math.floor(d / D) + 'd ago';
}

export function dur(ms: number) {
	const s = Math.max(0, Math.round(ms / S));
	if (s < 60) return s + 's';
	const m = Math.floor(s / 60);
	if (m < 60) return m + 'm ' + pad(s % 60) + 's';
	return Math.floor(m / 60) + 'h ' + pad(m % 60) + 'm';
}

export const clk = (ms: number) => {
	const s = Math.max(0, Math.floor(ms / S));
	return Math.floor(s / 60) + ':' + pad(s % 60);
};

export const hms = (t: Ms) => {
	const d = new Date(t);
	return pad(d.getHours()) + ':' + pad(d.getMinutes()) + ':' + pad(d.getSeconds());
};

/** a moment days away: `24 Nov, 15:00` */
export const day = (t: Ms) =>
	new Date(t).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' }) +
	', ' +
	hms(t).slice(0, 5);

export const short = (rev: string) => rev.slice(0, 7);

export function source(s: SourceView) {
	switch (s.kind) {
		case 'git':
			return s.git_ref ? `${s.git_ref}@${short(s.rev)}` : short(s.rev);
		case 'snapshot':
			return `snapshot ${s.label}`;
		case 'generation':
			return `generation ${s.number}`;
	}
}

export function origin(o: OriginView) {
	switch (o.kind) {
		case 'github':
			return `actions · ${o.workflow} #${o.run_id}`;
		case 'user':
			return `${o.user}@${o.host}`;
		case 'follow':
			return 'catch-up';
	}
}

export const plat = (s: MachineState) =>
	`${s.platform} · ${s.system.split('-')[0].replace('aarch64', 'arm64')}`;
