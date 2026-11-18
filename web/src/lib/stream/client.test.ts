import { describe, expect, test } from 'vitest';
import { answer, split } from './client.ts';

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

describe('answer', () => {
	const h = (o: Record<string, string> = {}) => new Headers(o);

	test('opens on success and sends a refused session to sign in', () => {
		expect(answer(200, h())).toBe('open');
		expect(answer(401, h())).toBe('refused');
	});

	test('reads a 530, or the offline Worker marked 503, as every host down', () => {
		expect(answer(530, h())).toBe('down');
		expect(answer(503, h({ 'coeus-offline': '1' }))).toBe('down');
	});

	test('takes any other failure for this request alone', () => {
		expect(answer(503, h())).toBe('failed');
		expect(answer(502, h())).toBe('failed');
	});
});
