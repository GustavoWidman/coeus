import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';
import { ParseError } from './parse.ts';
import { parseDiff, parseLogEvent } from './parse-log.ts';

const contract = JSON.parse(
	readFileSync(new URL('../../../../contract/web.json', import.meta.url), 'utf8')
) as { log: { kind: string }[]; diff: unknown };

describe('parseLogEvent', () => {
	test.each(contract.log.map((e) => [e.kind, e]))('reads the contract %s whole', (_, event) => {
		expect(parseLogEvent(JSON.stringify(event))).toEqual(event);
	});

	test('refuses a line of a kind it does not know', () => {
		const lines = structuredClone(contract.log.find((e) => e.kind === 'lines')) as any;
		lines.lines[0].kind = 'shout';
		expect(() => parseLogEvent(JSON.stringify(lines))).toThrow(ParseError);
	});
});

describe('parseDiff', () => {
	test('reads the contract diff whole', () => {
		expect(parseDiff(JSON.stringify(contract.diff))).toEqual(contract.diff);
	});

	test('refuses versions that are not a list', () => {
		const diff = structuredClone(contract.diff) as any;
		diff.changes[0].before = '1.0';
		expect(() => parseDiff(JSON.stringify(diff))).toThrow(ParseError);
	});
});
