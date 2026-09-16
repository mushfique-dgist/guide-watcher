import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  buildGenerateOptions,
  buildResumePrepOptions,
  PREP_PROVIDER,
  WRITER_PROVIDER,
} from './jobRequests.js';

const input = {
  prepProvider: 'codex-chatgpt',
  prepModel: 'gpt-5.6-luna',
  prepEffort: 'medium',
  collectionFallbacks: [
    { model: 'gpt-5.6-terra', effort: 'medium' },
    { model: 'gpt-5.6-sol', effort: 'medium' },
  ],
  writerProvider: 'claude-code',
  writerModel: 'claude-opus-4-8',
  writerEffort: 'high',
  fallbacks: [{ provider: 'codex-chatgpt', model: 'gpt-5.6-sol', effort: 'high' }],
  courseProfile: 'computer-networks',
  requireSlideCoverage: true,
  requireVisuals: true,
};

test('generate request has only the typed generate contract', () => {
  assert.deepEqual(buildGenerateOptions(input), {
    prep: { provider: 'codex-chatgpt', model: 'gpt-5.6-luna', effort: 'medium' },
    collectionFallbacks: [
      { provider: 'codex-chatgpt', model: 'gpt-5.6-terra', effort: 'medium' },
      { provider: 'codex-chatgpt', model: 'gpt-5.6-sol', effort: 'medium' },
    ],
    writer: { provider: 'claude-code', model: 'claude-opus-4-8', effort: 'high' },
    fallbacks: [{ provider: 'codex-chatgpt', model: 'gpt-5.6-sol', effort: 'high' }],
    courseProfile: 'computer-networks',
    requireSlideCoverage: true,
    requireVisuals: true,
  });
});

test('resume request carries exact prep and writer selections without course or gate fields', () => {
  const request = buildResumePrepOptions(input);
  assert.deepEqual(request, {
    prep: { provider: 'codex-chatgpt', model: 'gpt-5.6-luna', effort: 'medium' },
    collectionFallbacks: [
      { provider: 'codex-chatgpt', model: 'gpt-5.6-terra', effort: 'medium' },
      { provider: 'codex-chatgpt', model: 'gpt-5.6-sol', effort: 'medium' },
    ],
    writer: { provider: 'claude-code', model: 'claude-opus-4-8', effort: 'high' },
    fallbacks: [{ provider: 'codex-chatgpt', model: 'gpt-5.6-sol', effort: 'high' }],
  });
  for (const forbidden of ['courseProfile', 'requireVisuals', 'provider', 'model', 'effort']) {
    assert.equal(Object.hasOwn(request, forbidden), false);
  }
});

test('request builders lock collection to Codex and writing to Claude', () => {
  const tampered = {
    ...input,
    prepProvider: 'ollama-local',
    writerProvider: 'openai',
    fallbacks: [
      { provider: 'openai', model: 'opus', effort: 'max' },
      { provider: 'ollama-cloud', model: 'gpt-5.6-sol', effort: 'xhigh' },
    ],
  };

  const generate = buildGenerateOptions(tampered);
  assert.equal(generate.prep.provider, PREP_PROVIDER);
  assert.deepEqual(generate.collectionFallbacks.map(item => item.provider), [PREP_PROVIDER, PREP_PROVIDER]);
  assert.equal(generate.writer.provider, WRITER_PROVIDER);
  assert.deepEqual(
    generate.fallbacks.map(item => item.provider),
    [WRITER_PROVIDER, PREP_PROVIDER],
  );

  const resume = buildResumePrepOptions(tampered);
  assert.equal(resume.writer.provider, WRITER_PROVIDER);
  assert.deepEqual(resume.collectionFallbacks.map(item => item.provider), [PREP_PROVIDER, PREP_PROVIDER]);
  assert.deepEqual(
    resume.fallbacks.map(item => item.provider),
    [WRITER_PROVIDER, PREP_PROVIDER],
  );
});

test('confirmation UI exposes model and effort selection without credential or endpoint controls', () => {
  const source = readFileSync(new URL('./ConfirmPanel.svelte', import.meta.url), 'utf8');
  for (const retiredSurface of [
    "invoke('list_models'",
    'needs_api_key',
    'model_refresh',
    'default_base_url',
    'bind:value={apiKey}',
    'bind:value={baseUrl}',
    '<span>Provider</span>\n          <select',
  ]) {
    assert.equal(source.includes(retiredSurface), false, `retired UI surface found: ${retiredSurface}`);
  }
  assert.match(source, /Context collection/);
  assert.match(source, /Guide writing/);
  for (const id of ['prep-model', 'prep-effort', 'collection-fallback-1-model', 'collection-fallback-1-effort', 'collection-fallback-2-model', 'collection-fallback-2-effort', 'writer-model', 'writer-effort', 'claude-fallback-model', 'codex-fallback-model']) {
    assert.match(source, new RegExp(`id="${id}"`));
  }
});
