/**
 * A job's log as the log view shows it: which lines it keeps, how each one looks, and how many
 * rows of text a line wraps to, which the view uses to place lines it has not drawn.
 */
import type { LineKind, LogLine } from '#lib/wire/types.ts';

/** the most lines a log keeps; older ones go, and the rest keep their numbers */
export const KEEP = 20_000;
/** lines that may pile up past KEEP before the oldest go, so they go in batches */
const SLACK = 1_000;

export class Lines {
	all: LogLine[] = [];
	/** lines dropped from the front: the first kept line is number `gone + 1` */
	gone = 0;

	/** adds lines at the end, and answers how many it dropped from the front */
	add(lines: LogLine[]): number {
		for (const l of lines) this.all.push(l);
		if (this.all.length <= KEEP + SLACK) return 0;
		const drop = this.all.length - KEEP;
		this.all.splice(0, drop);
		this.gone += drop;
		return drop;
	}
}

const CLASS: Record<LineKind, string> = {
	coeus: '',
	phase: 'p',
	fetch: 'c',
	build: '',
	output: '',
	error: 'e',
	done: 'k',
	note: ''
};

/** a stretch of a line's text; `px` is the builder's name, `hs` a part cut short */
export interface Run {
	text: string;
	dim?: 'px' | 'hs';
}

const SHORTEN = /\/nix\/store\/([0-9a-z]{32})-| from 'https:\/\/cache\.nixos\.org'/g;

/**
 * How a line shows: its class, and its text in runs. coeus's own lines (those it stamped with
 * a time) say so, phases read as headings, and store hashes and the binary cache's address,
 * which repeat on every other line, shrink and dim.
 */
export function look(line: LogLine, prev?: LogLine): { cls: string; runs: Run[] } {
	let cls = CLASS[line.kind];
	// two phases in a row share the space between them, as the margins they stand for would
	if (line.kind === 'phase' && prev?.kind === 'phase') cls += ' pp';
	const runs: Run[] = [];
	let text =
		line.kind === 'phase'
			? `── ${line.text}`
			: line.at !== null
				? `coeus: ${line.text}`
				: line.text;
	if (line.prefix !== null) {
		runs.push({ text: `${line.prefix}>`, dim: 'px' });
		text = ' ' + text;
	}
	let at = 0;
	for (const m of text.matchAll(SHORTEN)) {
		if (m.index > at) runs.push({ text: text.slice(at, m.index) });
		runs.push({
			text: m[1] ? `/nix/store/${m[1].slice(0, 7)}…-` : ' from cache.nixos.org',
			dim: 'hs'
		});
		at = m.index + m[0].length;
	}
	if (at < text.length) runs.push({ text: text.slice(at) });
	return { cls, runs };
}

/**
 * How many rows `text` takes in a column `cols` characters wide, wrapped as the browser wraps
 * preformatted text: at spaces and after hyphens, and anywhere within a word too long for a
 * row. Spaces at the end of a row hang past it. The browser knows more places to break, so
 * this is an estimate until the line is drawn and measured.
 */
export function rows(text: string, cols: number): number {
	if (cols < 1 || text.length <= cols) return 1;
	let rows = 1;
	let col = 0;
	let i = 0;
	while (i < text.length) {
		let j = i;
		while (j < text.length && text[j] !== ' ' && text[j] !== '-') j++;
		if (j < text.length) j++;
		const len = j - i;
		const shown = text[j - 1] === ' ' ? len - 1 : len;
		if (col + shown > cols && col > 0) {
			rows++;
			col = 0;
		}
		let rest = len;
		while ((text[j - 1] === ' ' ? rest - 1 : rest) > cols) {
			rows++;
			rest -= cols;
		}
		col += rest;
		i = j;
	}
	return rows;
}

/** the length of a line's text as `look` shows it */
export const shown = (runs: Run[]) => runs.reduce((n, r) => n + r.text.length, 0);
