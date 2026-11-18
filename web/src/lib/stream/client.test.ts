import { describe, expect, test } from 'vitest';
import { split } from './client.ts';

describe('split', () => {
	test('reads each message with its id and data', () => {
		const { messages, rest } = split(
			'id: a.1\nevent: machine\ndata: {"x":1}\n\nid: a.2\nevent: progress\ndata: {"x":2}\n\n'
		);
		expect(messages).toEqual([
			{ id: 'a.1', data: '{"x":1}' },
			{ id: 'a.2', data: '{"x":2}' }
		]);
		expect(rest).toBe('');
	});

	test('keeps an unfinished message for the next read', () => {
		const first = split('id: a.1\ndata: {"x":1}\n\nid: a.2\nda');
		expect(first.messages).toEqual([{ id: 'a.1', data: '{"x":1}' }]);
		const second = split(first.rest + 'ta: {"x":2}\n\n');
		expect(second.messages).toEqual([{ id: 'a.2', data: '{"x":2}' }]);
	});

	test('skips comments, such as the heartbeat', () => {
		expect(split(':\n\n').messages).toEqual([]);
	});

	test('joins data lines, takes CRLF, and leaves a message without an id unnamed', () => {
		expect(split('data: a\r\ndata: b\r\n\r\n').messages).toEqual([{ id: null, data: 'a\nb' }]);
	});
});
