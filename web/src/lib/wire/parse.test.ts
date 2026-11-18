import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';
import { ParseError, parseFleetEvent } from './parse.ts';

const contract = JSON.parse(
	readFileSync(new URL('../../../../contract/web.json', import.meta.url), 'utf8')
) as { fleet: { kind: string }[] };

describe('parseFleetEvent', () => {
	test.each(contract.fleet.map((e) => [e.kind, e]))(
		'reads the contract %s whole, every field included',
		(_, event) => {
			expect(parseFleetEvent(JSON.stringify(event))).toEqual(event);
		}
	);

	const snapshot = () => structuredClone(contract.fleet.find((e) => e.kind === 'snapshot')!) as any;

	test('drops fields it does not know, which a newer server may send', () => {
		const event = snapshot();
		event.machines[0].mood = 'calm';
		const parsed = parseFleetEvent(JSON.stringify(event)) as any;
		expect(parsed.machines[0]).not.toHaveProperty('mood');
	});

	test.each([
		['an unknown kind', (e: any) => (e.kind = 'gossip')],
		['a missing field', (e: any) => delete e.machines[0].link],
		['a field of the wrong type', (e: any) => (e.machines[0].state.generation = '41')],
		['an unknown phase', (e: any) => (e.machines[0].state.running.state.phase = 'dreaming')],
		['an unknown link', (e: any) => (e.machines[0].link = { kind: 'telepathy' })],
		['a list that is not one', (e: any) => (e.deployments = {})],
		['an infinite number', (e: any) => (e.machines[0].state.seen_at = 1e400)]
	])('refuses %s', (_, spoil) => {
		const event = snapshot();
		spoil(event);
		expect(() => parseFleetEvent(JSON.stringify(event))).toThrow(ParseError);
	});

	test('refuses what is not JSON', () => {
		expect(() => parseFleetEvent('{"kind":')).toThrow(ParseError);
	});
});
