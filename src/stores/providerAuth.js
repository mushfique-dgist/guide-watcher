import { get, writable } from 'svelte/store';

const PROVIDERS = new Set(['codex-chatgpt', 'claude-code']);
const STATES = new Set(['starting', 'waiting', 'succeeded', 'failed', 'cancelled']);

export const providerAuthSessions = writable({});

function validSession(value) {
  return value
    && typeof value === 'object'
    && PROVIDERS.has(value.provider)
    && typeof value.sessionId === 'string'
    && value.sessionId.length > 0
    && STATES.has(value.state);
}

export function rememberProviderAuthStart(session) {
  if (!validSession(session) || session.state !== 'starting') return false;
  providerAuthSessions.update(current => {
    const existing = current[session.provider];
    if (existing?.sessionId === session.sessionId && existing.state !== 'starting') return current;
    return {
      ...current,
      [session.provider]: { ...session, message: 'Starting secure sign-in…' },
    };
  });
  return true;
}

export function applyProviderAuthEvent(update) {
  if (!validSession(update)) return false;
  providerAuthSessions.update(current => {
    const existing = current[update.provider];
    if (existing?.sessionId !== update.sessionId && ['starting', 'waiting'].includes(existing?.state)) {
      return current;
    }
    return { ...current, [update.provider]: { ...update } };
  });
  return true;
}

export function providerAuthSession(provider) {
  return get(providerAuthSessions)[provider] ?? null;
}
