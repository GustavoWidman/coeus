/**
 * Every state the dashboard can show, as data: where the simulated fleet's clock stands, which
 * page is open, what the network answers instead of the server, and what a person does before
 * the screenshot. `scripts/matrix.ts` runs each state in every variant below.
 *
 * Times are seconds on `coeus web --fixture instrument`'s timeline, which runs 108.2 s:
 *
 *   0     lab builds #61 (wave 2 of main@2cbe6da), #62 queued behind it; desktop-nixos failed
 *         #88 in wave 2; laptop-mac asleep; oracle-1 needs a reboot, oracle-2 a boot
 *   29.5  #61 activating, 33 verifying, 36 succeeded
 *   37.5  wave 3: altar runs #43 until 61; lab runs #62 (fetching 38, evaluating 41.5,
 *         building 47, activating 50.5, verifying 53, done 55)
 *   61    the rollout settles with laptop-mac pending
 *   67    laptop-mac wakes and catches up with #37 (building 80.5, done 94.5)
 *
 * Steps that need an element name it by the prototype's markup, which the screens keep.
 */

export type Screen = 'fleet' | 'machine' | 'deployment' | 'log' | 'signin' | 'down';
export type Theme = 'light' | 'dark';
export type Viewport = 'phone' | 'desktop';
export type Motion = 'full' | 'reduced';
export type Engine = 'chromium' | 'webkit';

/** An answer the page gets instead of the server's, for requests whose URL matches the glob. */
export type Net =
	| { url: string; hold: true }
	| { url: string; abort: true }
	| { url: string; status: number; headers?: Record<string, string>; body?: string };

/** A change to what the server streams, for states the simulated fleet never reaches. */
export type Rewrite =
	/** merged into the machine's card wherever the stream carries it */
	| { machine: string; set: Record<string, unknown> }
	| { deployments: 'none' }
	/** a log stream opens with this many extra lines */
	| { lines: number };

export type Step =
	| { click: string; text?: string }
	| { scroll: string; top: number }
	/** runs the fleet's clock this many seconds, then holds it */
	| { advance: number }
	| { reload: true }
	| { net: Net }
	/** the server answers requests matching this glob again */
	| { restore: string }
	| { wait: number };

export interface State {
	screen: Screen;
	name: string;
	/** the page opened */
	path: string;
	/** where the fleet's clock stands for the screenshot */
	t: number;
	/** where a 10 s window of live updates starts; states where nothing moves have none */
	live?: number;
	net?: Net[];
	rewrite?: Rewrite[];
	steps?: Step[];
	/**
	 * The same state in the prototype: the screen its `#s=` opens, more of its hash, then
	 * screens to open with its `go()`. Steps run as above. `null` where it has no such state.
	 */
	reference: { s: string; hash?: string; go?: string[] } | null;
}

export interface Cell {
	id: string;
	state: State;
	theme: Theme;
	viewport: Viewport;
	motion: Motion;
}

const ROLLOUT = '2d2eef4459f7ebbf';
const SNAPSHOT = '7fb4af20b514a4a9';

const STREAM = '**/api/stream';
const API = '**/api/**';
const hold = (url: string): Net => ({ url, hold: true });
const down: Net = { url: API, status: 530, body: '' };
const worker: Net = {
	url: API,
	status: 503,
	headers: { 'content-type': 'application/json', 'coeus-offline': '1', 'retry-after': '15' },
	body: '{"offline":true}'
};
// Signing in. A simulated fleet that lets anyone read it offers no sign-in address, so these
// answer for one: lab's, on the tailnet.
const START = 1_794_927_600_000;
const LAB = 'https://lab.tail4a3ea.ts.net:45730';
const cors = { 'access-control-allow-origin': '*' };
const json = { ...cors, 'content-type': 'application/json' };
const hello: Net = {
	url: '**/api/hello',
	status: 200,
	headers: json,
	body: JSON.stringify({ signin: [LAB], build: '', schema: 1 })
};
const ping: Record<'up' | 'off', Net> = {
	up: { url: '**/auth/ping', status: 204, headers: cors },
	off: { url: '**/auth/ping', abort: true }
};
const claims = btoa(JSON.stringify({ v: 1, iat: START, exp: START + 7 * 864e5 }));
const session: Record<'ok' | 'refused', Net> = {
	ok: {
		url: '**/auth/session',
		status: 200,
		headers: json,
		body: JSON.stringify({
			token: `v1.${claims.replace(/=+$/, '')}.mac`,
			viewer: { login: 'gustavo@github', device: 'iphone', expires_at: START + 7 * 864e5 }
		})
	},
	refused: {
		url: '**/auth/session',
		status: 403,
		headers: cors,
		body: 'the tailnet policy grants no access'
	}
};

/** the machine screen of `host` at `t`, with the prototype's machine screen for it */
const machine = (name: string, host: string, t: number, extra: Partial<State> = {}): State => ({
	screen: 'machine',
	name,
	path: `/machines/${host}`,
	t,
	reference: { s: 'fleet', go: [`m:${host}`] },
	...extra
});

const log = (name: string, host: string, job: number, extra: Partial<State> = {}): State => ({
	screen: 'log',
	name,
	path: `/machines/${host}/jobs/${job}`,
	t: 10,
	reference: { s: 'fleet', go: [`m:${host}`, `log:${host}:${job}`] },
	...extra
});

export const STATES: State[] = [
	// fleet
	{
		screen: 'fleet',
		name: 'cold skeleton',
		path: '/',
		t: 0,
		net: [hold(STREAM)],
		steps: [{ wait: 800 }],
		reference: { s: 'fleet', hash: 'cold=60000' }
	},
	{
		screen: 'fleet',
		name: 'cached connecting',
		path: '/',
		t: 0,
		steps: [{ wait: 1000 }, { net: hold(STREAM) }, { reload: true }, { wait: 500 }],
		reference: null
	},
	{ screen: 'fleet', name: 'idle', path: '/', t: 100, reference: { s: 'fleet' } },
	{
		screen: 'fleet',
		name: 'building',
		path: '/',
		t: 10,
		live: 10,
		reference: { s: 'fleet' }
	},
	{
		screen: 'fleet',
		name: 'job finishing',
		path: '/',
		t: 36.2,
		live: 33,
		reference: { s: 'fleet' }
	},
	{
		screen: 'fleet',
		name: 'machine waking',
		path: '/',
		t: 67.2,
		live: 64,
		reference: { s: 'fleet' }
	},
	{
		screen: 'fleet',
		name: 'wave advancing',
		path: '/',
		t: 37.6,
		live: 35,
		reference: { s: 'fleet' }
	},
	{
		screen: 'fleet',
		name: 'deploy done',
		path: '/',
		t: 61.2,
		live: 58,
		reference: { s: 'fleet' }
	},
	{
		screen: 'fleet',
		name: 'host unreachable',
		path: '/',
		t: 10,
		rewrite: [
			{
				machine: 'oracle-2',
				set: {
					link: {
						kind: 'unreachable',
						since: 1794927560000,
						reason: { kind: 'failed', detail: 'connection timed out' }
					}
				}
			}
		],
		reference: null
	},
	{
		screen: 'fleet',
		name: 'host refused',
		path: '/',
		t: 10,
		rewrite: [
			{
				machine: 'oracle-2',
				set: {
					link: {
						kind: 'unreachable',
						since: 1794927560000,
						reason: { kind: 'refused', detail: 'this key may not watch' }
					}
				}
			}
		],
		reference: null
	},
	{
		screen: 'fleet',
		name: 'no deployment',
		path: '/',
		t: 100,
		rewrite: [{ deployments: 'none' }],
		reference: null
	},

	// machine
	machine('preparing', 'lab', 37.6, { live: 37.5 }),
	machine('fetching', 'lab', 39, { live: 38 }),
	machine('evaluating', 'lab', 44, { live: 42 }),
	machine('building', 'lab', 10, { live: 10 }),
	machine('activating', 'lab', 30, { live: 29.5 }),
	machine('verifying', 'lab', 34, { live: 33 }),
	machine('queued next', 'laptop-mac', 67.5),
	machine('idle', 'altar', 10),
	machine('asleep', 'laptop-mac', 10),
	machine('failed recently', 'desktop-nixos', 10),
	machine('follow paused', 'altar', 10, {
		rewrite: [
			{
				machine: 'altar',
				set: {
					follow: {
						git_ref: 'deployed',
						latest: {
							rev: '2cbe6da7e56b76230b636eccac43a797985e81e1',
							committed_at: 1794927048000
						},
						paused: true
					}
				}
			}
		],
		reference: null
	}),
	machine('pending boot', 'oracle-2', 10),
	machine('reboot required', 'oracle-1', 10),

	// deployment
	{
		screen: 'deployment',
		name: 'wave 1 running',
		path: `/deployments/${SNAPSHOT}`,
		t: 44,
		live: 42,
		reference: null
	},
	{
		screen: 'deployment',
		name: 'mid-rollout with a failure',
		path: `/deployments/${ROLLOUT}`,
		t: 10,
		live: 10,
		reference: { s: 'deploy' }
	},
	{
		screen: 'deployment',
		name: 'all done',
		path: `/deployments/${ROLLOUT}`,
		t: 95,
		reference: { s: 'deploy' }
	},
	{
		screen: 'deployment',
		name: 'pending catch-up',
		path: `/deployments/${ROLLOUT}`,
		t: 62,
		reference: { s: 'deploy' }
	},
	{
		screen: 'deployment',
		name: 'diff expanded',
		path: `/deployments/${ROLLOUT}`,
		t: 10,
		steps: [{ click: '.vrow', text: 'desktop-nixos' }],
		reference: { s: 'deploy' }
	},
	{
		screen: 'deployment',
		name: 'diff loading',
		path: `/deployments/${ROLLOUT}`,
		t: 10,
		net: [hold('**/api/machines/*/jobs/*/diff')],
		steps: [{ click: '.vrow', text: 'desktop-nixos' }],
		reference: null
	},

	// log
	log('tailing', 'lab', 61, { live: 10 }),
	log('scrolled up', 'lab', 61, {
		steps: [{ scroll: '.logbody', top: 0 }, { advance: 3 }]
	}),
	log('finished ok', 'lab', 60),
	log('finished failed', 'desktop-nixos', 88),
	log('empty', 'lab', 62),
	log('20k lines', 'lab', 61, { rewrite: [{ lines: 20_000 }], reference: null }),

	// sign-in
	{
		screen: 'signin',
		name: 'off the tailnet',
		path: '/signin',
		t: 10,
		net: [hello, ping.off],
		steps: [{ wait: 500 }],
		reference: { s: 'signin' }
	},
	{
		screen: 'signin',
		name: 'signing',
		path: '/signin',
		t: 10,
		net: [hello, ping.up, hold('**/auth/session')],
		steps: [{ wait: 500 }],
		reference: null
	},
	{
		screen: 'signin',
		name: 'signed in',
		path: '/signin',
		t: 10,
		net: [hello, ping.up, session.ok],
		steps: [{ wait: 500 }],
		reference: { s: 'signedin' }
	},
	{
		// signed in, then off the tailnet when the server stops taking the session
		screen: 'signin',
		name: 'session expired',
		path: '/signin',
		t: 10,
		net: [hello, ping.up, session.ok],
		steps: [
			{ wait: 500 },
			{ net: ping.off },
			{ net: { url: STREAM, status: 401, body: '' } },
			{ click: '.btn' },
			{ wait: 500 }
		],
		reference: null
	},
	{
		screen: 'signin',
		name: 'grant refused',
		path: '/signin',
		t: 10,
		net: [hello, ping.up, session.refused],
		steps: [{ wait: 500 }],
		reference: null
	},

	// all hosts down, over any screen
	{
		screen: 'down',
		name: 'from cache',
		path: '/',
		t: 10,
		net: [down],
		reference: { s: 'offline' }
	},
	{
		screen: 'down',
		name: 'from the worker',
		path: '/',
		t: 10,
		net: [worker],
		reference: null
	},
	{
		screen: 'down',
		name: 'recovering',
		path: '/',
		t: 10,
		net: [down],
		steps: [{ wait: 500 }, { restore: API }, { wait: 16_000 }],
		reference: null
	}
];

const THEMES: Theme[] = ['light', 'dark'];
const VIEWPORTS: Viewport[] = ['phone', 'desktop'];
const MOTIONS: Motion[] = ['full', 'reduced'];

export const slug = (s: string) => s.replace(/[^a-z0-9]+/gi, '-').toLowerCase();

/** every state in every variant */
export function cells(states = STATES): Cell[] {
	return states.flatMap((state) =>
		THEMES.flatMap((theme) =>
			VIEWPORTS.flatMap((viewport) =>
				MOTIONS.map((motion) => ({
					id: [state.screen, slug(state.name), theme, viewport, motion].join('.'),
					state,
					theme,
					viewport,
					motion
				}))
			)
		)
	);
}
