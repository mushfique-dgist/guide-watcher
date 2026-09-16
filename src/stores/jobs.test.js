import test, { beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { get } from 'svelte/store';

import {
  jobs,
  activeJobs,
  pendingFiles,
  pendingSource,
  showConfirmPanel,
  deferredWatcherFiles,
  addJob,
  appendStep,
  receiveWatcherFiles,
  revealDeferredWatcherFiles,
  updateJobActivity,
  updateJobStatus,
  mergeHistory,
} from './jobs.js';

beforeEach(() => {
  jobs.set([]);
  pendingFiles.set([]);
  pendingSource.set('watcher');
  showConfirmPanel.set(false);
  deferredWatcherFiles.set([]);
});

function metadata(id) {
  return {
    id,
    filename: 'lecture.pdf',
    filepath: 'C:/course/lecture.pdf',
    folder: 'course',
    outputName: 'lecture_Guide.md',
    status: 'starting',
    activity: 'Preparing source text locally…',
    provider: 'codex-chatgpt',
    model: 'gpt-5.6-sol',
    effort: 'xhigh',
    prepProvider: 'codex-chatgpt',
    prepModel: 'gpt-5.6-sol',
    prepEffort: 'xhigh',
    fallbackChain: [
      { provider: 'claude-code', model: 'opus', effort: 'max' },
      { provider: 'codex-chatgpt', model: 'gpt-5.6-sol', effort: 'xhigh' },
    ],
    courseProfile: 'computer-networks',
    requireSlideCoverage: true,
    requireVisuals: true,
    startedAt: 1,
    finishedAt: null,
  };
}

test('buffers backend events that arrive before frontend job metadata', () => {
  updateJobStatus('race-job', 'working');
  updateJobActivity('race-job', 'Collecting context');
  appendStep('race-job', { type: 'phase', text: 'Phase 1' });
  addJob(metadata('race-job'));

  const [job] = get(jobs);
  assert.equal(job.placeholder, false);
  assert.equal(job.filename, 'lecture.pdf');
  assert.equal(job.status, 'working');
  assert.equal(job.activity, 'Collecting context');
  assert.deepEqual(job.steps.map(step => step.text), ['Phase 1']);
});

test('watcher files are deferred without corrupting an open resume flow', () => {
  pendingFiles.set(['C:/course/incomplete_Guide.md']);
  pendingSource.set('resume');
  showConfirmPanel.set(true);

  receiveWatcherFiles([
    'C:/course/Lecture 02.pdf',
    'c:\\COURSE\\lecture 02.PDF',
  ]);

  assert.deepEqual(get(pendingFiles), ['C:/course/incomplete_Guide.md']);
  assert.equal(get(pendingSource), 'resume');
  assert.deepEqual(get(deferredWatcherFiles), ['C:/course/Lecture 02.pdf']);

  pendingFiles.set([]);
  showConfirmPanel.set(false);
  revealDeferredWatcherFiles();

  assert.deepEqual(get(pendingFiles), ['C:/course/Lecture 02.pdf']);
  assert.equal(get(pendingSource), 'watcher');
  assert.equal(get(showConfirmPanel), true);
  assert.deepEqual(get(deferredWatcherFiles), []);
});

test('retains unique monotonic step ids after the 500-step cap', () => {
  addJob(metadata('long-job'));
  for (let index = 0; index < 505; index += 1) {
    appendStep('long-job', { type: 'raw', text: `line ${index}` });
  }

  const [job] = get(jobs);
  const ids = job.steps.map(step => step.id);
  assert.equal(job.steps.length, 500);
  assert.equal(new Set(ids).size, 500);
  assert.equal(ids[0], 5);
  assert.equal(ids.at(-1), 504);
  assert.equal(job.nextStepId, 505);
});

test('merged text does not consume ids or double count token deltas', () => {
  addJob(metadata('text-job'));
  appendStep('text-job', {
    type: 'text_chunk',
    text: 'one',
    tokenDelta: { inputTokens: 4, outputTokens: 2 },
  });
  appendStep('text-job', { type: 'text_chunk', text: ' two' });
  appendStep('text-job', { type: 'phase', text: 'Next' });

  const [job] = get(jobs);
  assert.equal(job.steps[0].content, 'one two');
  assert.deepEqual(job.steps.map(step => step.id), [0, 1]);
  assert.equal(job.stats.inputTokens, 4);
  assert.equal(job.stats.outputTokens, 2);
  assert.equal(job.stats.charsGenerated, 7);
});

test('duplicate lifecycle events are idempotent', () => {
  addJob(metadata('duplicate-job'));

  appendStep('duplicate-job', { type: 'phase', text: 'Phase 1: context collection' });
  appendStep('duplicate-job', { type: 'phase', text: 'Phase 1: context collection' });
  appendStep('duplicate-job', {
    type: 'complete',
    inputTokens: 12,
    outputTokens: 3,
    cacheRead: 0,
    cacheWrite: 0,
  });
  appendStep('duplicate-job', {
    type: 'complete',
    inputTokens: 12,
    outputTokens: 3,
    cacheRead: 0,
    cacheWrite: 0,
  });

  const [job] = get(jobs);
  assert.deepEqual(job.steps.map(step => step.type), ['phase', 'complete']);
  assert.deepEqual(job.steps.map(step => step.id), [0, 1]);
  assert.equal(job.nextStepId, 2);
  assert.equal(job.stats.inputTokens, 12);
  assert.equal(job.stats.outputTokens, 3);
});

test('late started events never turn terminal runs active again',()=>{addJob(metadata('terminal'));updateJobStatus('terminal','failed',100);updateJobStatus('terminal','working');assert.equal(get(jobs)[0].status,'failed');assert.equal(get(jobs)[0].finishedAt,100);});
test('durable refresh preserves streamed details and newer terminal results',()=>{addJob(metadata('saved'));appendStep('saved',{type:'phase',text:'Research'});updateJobStatus('saved','done',100);mergeHistory([{id:'saved',filename:'saved.pdf',status:'working',startedAt:1,finishedAt:null,summary:'Working'}]);assert.equal(get(jobs)[0].status,'done');assert.equal(get(jobs)[0].finishedAt,100);assert.equal(get(jobs)[0].steps[0].text,'Research');mergeHistory([{id:'saved',filename:'saved.pdf',status:'done',startedAt:1,finishedAt:100}]);assert.equal(get(jobs).length,1);});
test('long streaming responses stay bounded without losing character totals',()=>{addJob(metadata('long'));for(let i=0;i<100;i++)appendStep('long',{type:'text_chunk',text:'x'.repeat(2000)});const job=get(jobs)[0];assert.equal(job.stats.charsGenerated,200000);assert(job.steps[0].content.length<=64000);});

test('specific terminal outcomes survive generic failure events',()=>{for(const status of ['cancelled','blocked','interrupted']){jobs.set([]);addJob(metadata(status));updateJobStatus(status,status,100);updateJobStatus(status,'failed',101);assert.equal(get(jobs)[0].status,status);assert.equal(get(jobs)[0].finishedAt,100);}});
test('stale active snapshots cannot replace terminal summary or recovery flags',()=>{mergeHistory([{id:'saved',status:'done',summary:'Published',canOpenOutput:true,startedAt:1,finishedAt:100}]);mergeHistory([{id:'saved',status:'working',summary:'Reading sources',canOpenOutput:false,startedAt:1}]);assert.equal(get(jobs)[0].summary,'Published');assert.equal(get(jobs)[0].canOpenOutput,true);});

test('queued jobs remain visible among active work',()=>{addJob({...metadata('queued'),status:'queued'});assert.equal(get(activeJobs).length,1);});

test('expanded preflight requests disappear when absent from the next durable snapshot',()=>{mergeHistory([{id:'request',status:'starting',startedAt:1}]);addJob(metadata('live'));mergeHistory([{id:'planned',status:'queued',startedAt:1}]);assert.deepEqual(get(jobs).map(job=>job.id).sort(),['live','planned']);});

test('watcher arrivals can be retained without replacing a modal view', () => {
  receiveWatcherFiles(['C:/course/new.pdf'], { reveal: false });
  assert.deepEqual(get(pendingFiles), ['C:/course/new.pdf']);
  assert.equal(get(showConfirmPanel), false);
  receiveWatcherFiles(['C:/course/next.pdf']);
  assert.deepEqual(get(pendingFiles), ['C:/course/new.pdf', 'C:/course/next.pdf']);
  assert.equal(get(showConfirmPanel), true);
});
