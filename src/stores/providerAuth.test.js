import assert from 'node:assert/strict';
import test from 'node:test';
import {
  applyProviderAuthEvent,
  providerAuthSession,
  providerAuthSessions,
  rememberProviderAuthStart,
} from './providerAuth.js';

test.beforeEach(() => providerAuthSessions.set({}));

test('provider sign-in state survives view remounts without regressing an early device event', () => {
  assert.equal(applyProviderAuthEvent({
    provider: 'codex-chatgpt',
    sessionId: 'one',
    state: 'waiting',
    message: 'Enter the code.',
    verificationUri: 'https://auth.openai.com/codex/device',
    userCode: 'ABCD-EFGH',
  }), true);
  assert.equal(rememberProviderAuthStart({
    provider: 'codex-chatgpt',
    sessionId: 'one',
    state: 'starting',
  }), true);

  assert.equal(providerAuthSession('codex-chatgpt').state, 'waiting');
  assert.equal(providerAuthSession('codex-chatgpt').userCode, 'ABCD-EFGH');
});

test('stale and malformed provider events cannot replace an active sign-in', () => {
  rememberProviderAuthStart({ provider: 'claude-code', sessionId: 'current', state: 'starting' });
  applyProviderAuthEvent({
    provider: 'claude-code',
    sessionId: 'stale',
    state: 'cancelled',
    message: 'Old session ended.',
  });
  assert.equal(providerAuthSession('claude-code').sessionId, 'current');
  assert.equal(applyProviderAuthEvent({ provider: 'unknown', sessionId: 'x', state: 'waiting' }), false);
  assert.equal(applyProviderAuthEvent({ provider: 'claude-code', sessionId: '', state: 'waiting' }), false);
});

test('a terminal provider session can be replaced by the next real sign-in', () => {
  applyProviderAuthEvent({
    provider: 'claude-code',
    sessionId: 'finished',
    state: 'succeeded',
    message: 'Connected.',
  });
  rememberProviderAuthStart({ provider: 'claude-code', sessionId: 'next', state: 'starting' });
  assert.equal(providerAuthSession('claude-code').sessionId, 'next');
  assert.equal(providerAuthSession('claude-code').state, 'starting');
});
