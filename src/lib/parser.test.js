import test from 'node:test';
import assert from 'node:assert/strict';

import { parseLine, parseLineEvents } from './parser.js';

test('preserves every Claude content block in order', () => {
  const raw = JSON.stringify({
    type: 'assistant',
    message: {
      usage: { input_tokens: 12, output_tokens: 7 },
      content: [
        { type: 'tool_use', name: 'Read', input: { file_path: 'C:/notes/week 1.pdf' } },
        { type: 'text', text: 'Collected the lecture.' },
      ],
    },
  });

  const events = parseLineEvents(raw);

  assert.equal(events.length, 2);
  assert.equal(events[0].type, 'tool_start');
  assert.equal(events[0].detail, 'week 1.pdf');
  assert.deepEqual(events[0].tokenDelta, { inputTokens: 12, outputTokens: 7 });
  assert.deepEqual(events[1], { type: 'text_chunk', text: 'Collected the lecture.' });
  assert.deepEqual(parseLine(raw), events[0]);
});

test('returns a token-only event when an assistant turn has no visible blocks', () => {
  const raw = JSON.stringify({
    type: 'assistant',
    message: { usage: { input_tokens: 3, output_tokens: 0 }, content: [] },
  });

  assert.deepEqual(parseLineEvents(raw), [
    { type: 'token_delta', inputTokens: 3, outputTokens: 0 },
  ]);
});

test('filters optional connector auth noise without hiding real errors', () => {
  assert.deepEqual(parseLineEvents('__stderr__Missing or invalid access token'), [
    { type: 'system', text: 'Optional connector auth unavailable; continuing without it.' },
  ]);
  assert.deepEqual(parseLineEvents('__stderr__fatal error: model unavailable'), [
    { type: 'raw', text: 'fatal error: model unavailable', isError: true },
  ]);
});

test('parses Codex failures and ignores structural noise', () => {
  assert.deepEqual(parseLineEvents('{"type":"turn.failed","error":{"message":"quota exceeded"}}'), [
    { type: 'raw', text: 'quota exceeded', isError: true },
  ]);
  assert.deepEqual(parseLineEvents('{"type":"item.started","item":{"type":"unknown"}}'), []);
});
