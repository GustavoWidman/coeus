/**
 * Fails when a stylesheet animates anything but transform, opacity or clip-path, the
 * properties a phone composites without laying out or repainting the page. It reads every
 * `@keyframes` block and every `transition` and `transition-property` declaration. A
 * transition that names no property animates all of them, and so fails too. The few
 * exceptions below name where they apply and why they are cheap; one that no longer
 * matches anything fails as well, so the list never outlives its reason.
 *
 *   bun scripts/motion-lint.ts [file or directory ...]   (default: build/)
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import postcss, { type AtRule, type Declaration, type Node, type Rule } from 'postcss';

export const ALLOWED = new Set([
	'transform',
	'translate',
	'scale',
	'rotate',
	'opacity',
	'clip-path'
]);

export const EXCEPTIONS: { at: string; property: string; why: string }[] = [
	{
		at: '.tap',
		property: 'background-color',
		why: 'the press tint, painted on one element only while a finger is down'
	},
	{
		at: '@keyframes stroke',
		property: 'stroke-dashoffset',
		why: 'the signed-in check mark, drawn once on a 52 px SVG'
	}
];

/** Words in a transition entry that are not the property it names. */
const TIMING =
	/^(?:[\d.]+m?s|ease|ease-in|ease-out|ease-in-out|linear|step-start|step-end|allow-discrete|normal|(?:var|cubic-bezier|steps|linear)\(.*\))$/;

/** Splits at `sep` outside parentheses. */
function split(value: string, sep: RegExp): string[] {
	const out: string[] = [];
	let depth = 0;
	let cur = '';
	for (const ch of value) {
		if (ch === '(') depth++;
		if (ch === ')') depth--;
		if (depth === 0 && sep.test(ch)) {
			if (cur.trim()) out.push(cur.trim());
			cur = '';
		} else cur += ch;
	}
	if (cur.trim()) out.push(cur.trim());
	return out;
}

/** The properties a transition declaration animates. */
function transitioned(decl: Declaration): string[] {
	const entries = split(decl.value, /,/);
	if (decl.prop.endsWith('transition-property')) return entries.map((e) => e.toLowerCase());
	return entries.map(
		(entry) =>
			split(entry, /\s/)
				.find((word) => !TIMING.test(word))
				?.toLowerCase() ?? 'all'
	);
}

function* cssFiles(path: string): Generator<string> {
	if (statSync(path).isDirectory()) {
		for (const name of readdirSync(path)) yield* cssFiles(join(path, name));
	} else if (path.endsWith('.css')) yield path;
}

const inside = (decl: Declaration) => {
	for (let n = decl.parent as Node | undefined; n; n = n.parent as Node | undefined) {
		if (n.type === 'atrule' && /keyframes$/i.test((n as AtRule).name)) return n as AtRule;
	}
	return undefined;
};

const used = new Set<(typeof EXCEPTIONS)[number]>();
function excepted(at: string[], property: string) {
	const hit = EXCEPTIONS.find((e) => e.property === property && at.every((a) => a === e.at));
	if (hit) used.add(hit);
	return !!hit;
}

// scripts/matrix.ts imports the rules above and holds the animations a page runs to them.
if (import.meta.main) lint();

function lint() {
	const targets = process.argv.slice(2);
	const files = (targets.length ? targets : ['build']).flatMap((t) => [...cssFiles(resolve(t))]);
	if (!files.length) throw new Error(`no stylesheets under ${targets.join(' ') || 'build'}`);

	const problems: string[] = [];
	for (const file of files) {
		postcss.parse(readFileSync(file, 'utf8'), { from: file }).walkDecls((decl) => {
			const keyframes = inside(decl);
			let at: string[];
			let animated: string[];
			if (keyframes) {
				if (decl.prop === 'animation-timing-function') return;
				at = [`@keyframes ${keyframes.params}`];
				animated = [decl.prop.toLowerCase()];
			} else if (/^(?:-webkit-)?transition(?:-property)?$/i.test(decl.prop)) {
				if (decl.value.trim() === 'none') return;
				const rule = decl.parent?.type === 'rule' ? (decl.parent as Rule) : undefined;
				at = rule ? split(rule.selector, /,/) : ['?'];
				animated = transitioned(decl);
			} else return;
			for (const property of animated) {
				if (ALLOWED.has(property) || excepted(at, property)) continue;
				problems.push(`${file}: ${at.join(', ')} animates ${property}`);
			}
		});
	}
	for (const e of EXCEPTIONS) {
		if (!used.has(e)) problems.push(`exception for ${e.property} at ${e.at} matches nothing`);
	}

	if (problems.length) {
		console.error(problems.join('\n'));
		console.error(
			`\n${problems.length} problem(s). Animate transform, opacity or clip-path instead.`
		);
		process.exit(1);
	}
	console.log(`motion: ${files.length} stylesheet(s) animate only composited properties`);
}
