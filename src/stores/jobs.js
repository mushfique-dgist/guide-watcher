import { isActive } from '../lib/ui.js';
import { writable, derived, get } from 'svelte/store';

export const jobs = writable([]);
export const selectedJobId = writable(null);
export const pendingFiles = writable([]);
export const showConfirmPanel = writable(false);
export const pendingSource = writable('watcher');  // 'watcher' | 'scan' | 'resume'
export const deferredWatcherFiles = writable([]);

const STEP_CAP = 500;
const deletedHistoryIds = new Set();

export function forgetHistoryJob(jobId) {
  deletedHistoryIds.add(jobId);
  jobs.update(list => list.filter(job => job.id !== jobId));
  if(get(selectedJobId) === jobId) selectedJobId.set(null);
}

// Empty stats object — populated as events stream in
function emptyStats() {
  return {
    inputTokens: 0,
    outputTokens: 0,
    cacheRead: 0,
    cacheWrite: 0,
    costUsd: null,      // set from final result event
    durationMs: null,
    numTurns: null,
    toolCounts: {},     // { Read: 3, Bash: 2, Write: 1, ... }
    charsGenerated: 0,  // total chars in text_block content
    guideBytes: null,   // set after file is written (future)
  };
}

function placeholderJob(id) {
  return {
    id,
    filename: 'Starting job…',
    filepath: '',
    folder: '',
    outputName: '',
    status: 'starting',
    activity: 'Waiting for job details…',
    provider: '',
    model: '',
    effort: '',
    prepProvider: '',
    prepModel: '',
    prepEffort: '',
    collectionFallbackChain: [],
    fallbackChain: [],
    courseProfile: 'auto',
    requireSlideCoverage: true,
    requireVisuals: true,
    startedAt: Date.now(),
    finishedAt: null,
    steps: [],
    stats: emptyStats(),
    nextStepId: 0,
    placeholder: true,
  };
}

function updateExistingOrPlaceholder(list, jobId, update) {
  if(deletedHistoryIds.has(jobId)) return list;
  let found = false;
  const next = list.map(job => {
    if (job.id !== jobId) return job;
    found = true;
    return update(job);
  });
  return found ? next : [update(placeholderJob(jobId)), ...next];
}

export const selectedJob = derived(
  [jobs, selectedJobId],
  ([$jobs, $id]) => $jobs.find(j => j.id === $id) ?? null
);

export const activeJobs = derived(jobs, $jobs =>
  $jobs.filter(j => isActive(j.status))
);

export const recentJobs = derived(jobs, $jobs => $jobs.filter(j => !j.archivedAt));

export const completedJobs = derived(jobs, $jobs =>
  $jobs.filter(j => ['done','done-warnings','failed','cancelled','interrupted','blocked'].includes(j.status))
);

// Cumulative totals across all completed jobs this session
export const sessionTotals = derived(completedJobs, $jobs =>
  $jobs.reduce((acc, j) => ({
    inputTokens:  acc.inputTokens  + j.stats.inputTokens,
    outputTokens: acc.outputTokens + j.stats.outputTokens,
    cacheRead:    acc.cacheRead    + j.stats.cacheRead,
    costUsd:      acc.costUsd      + (j.stats.costUsd ?? 0),
    jobCount:     acc.jobCount     + 1,
  }), { inputTokens: 0, outputTokens: 0, cacheRead: 0, costUsd: 0, jobCount: 0 })
);

function fileKey(fp) {
  return fp.replace(/\\/g, '/').toLowerCase();
}

function appendUnique(current, files) {
  const seen = new Set(current.map(fileKey));
  const next = [...current];
  for (const fp of files ?? []) {
    const key = fileKey(fp);
    if (!seen.has(key)) {
      seen.add(key);
      next.push(fp);
    }
  }
  return next;
}

export function appendPendingFiles(files) {
  if (!files?.length) return;
  pendingFiles.update(current => appendUnique(current, files));
}

export function deferWatcherFiles(files) {
  deferredWatcherFiles.update(current => appendUnique(current, files));
}

export function receiveWatcherFiles(files, { reveal = true } = {}) {
  if (!files?.length) return;
  if (get(pendingSource) === 'resume' && get(pendingFiles).length) {
    deferredWatcherFiles.update(current => appendUnique(current, files));
    return;
  }
  pendingSource.set('watcher');
  appendPendingFiles(files);
  if (reveal) showConfirmPanel.set(true);
}

export function revealDeferredWatcherFiles() {
  const deferred = get(deferredWatcherFiles);
  if (!deferred.length) return;
  deferredWatcherFiles.set([]);
  pendingSource.set('watcher');
  appendPendingFiles(deferred);
  showConfirmPanel.set(true);
}

export function addJob(job) {
  if(deletedHistoryIds.has(job.id)) return;
  jobs.update(list => {
    const existing = list.find(item => item.id === job.id);
    if (!existing) {
      return [{ ...job, steps: [], stats: emptyStats(), nextStepId: 0, placeholder: false }, ...list];
    }

    const backendAlreadyStarted = existing.status !== 'starting' || existing.steps.length > 0;
    const merged = {
      ...existing,
      ...job,
      status: backendAlreadyStarted ? existing.status : job.status,
      activity: backendAlreadyStarted ? existing.activity : job.activity,
      finishedAt: existing.finishedAt ?? job.finishedAt,
      steps: existing.steps,
      stats: existing.stats,
      nextStepId: existing.nextStepId,
      placeholder: false,
    };
    return list.map(item => item.id === job.id ? merged : item);
  });
}

export function updateJobStatus(jobId, status, finishedAt = null) {
  jobs.update(list =>
    updateExistingOrPlaceholder(list, jobId, j => {
      const terminal = ['done','done-warnings','failed','cancelled','interrupted','blocked'].includes(j.status);
      if (terminal && (['starting','working','queued'].includes(status) || (status === 'failed' && ['cancelled','blocked','interrupted'].includes(j.status)))) return j;
      return { ...j, status, finishedAt: finishedAt ?? j.finishedAt };
    })
  );
}

export function updateJobActivity(jobId, activity) {
  jobs.update(list =>
    updateExistingOrPlaceholder(list, jobId, j => ({ ...j, activity }))
  );
}

export function appendStep(jobId, step) {
  if (!step) return;
  jobs.update(list =>
    updateExistingOrPlaceholder(list, jobId, j => {
      let steps = j.steps;
      let stats = j.stats;
      let nextStepId = j.nextStepId ?? 0;
      const nextId = nextStepId;

      // ── Live token delta from any event that carries one ──
      if (step.tokenDelta) {
        stats = {
          ...stats,
          inputTokens:  stats.inputTokens  + (step.tokenDelta.inputTokens  || 0),
          outputTokens: stats.outputTokens + (step.tokenDelta.outputTokens || 0),
        };
      }

      // ── Pure token_delta (no visual step) ─────────────────
      if (step.type === 'token_delta') {
        return { ...j, stats };
      }

      // ── Final result — override with authoritative totals ──
      if (step.type === 'complete') {
        stats = {
          ...stats,
          inputTokens:  step.inputTokens  ?? stats.inputTokens,
          outputTokens: step.outputTokens ?? stats.outputTokens,
          cacheRead:    step.cacheRead    ?? stats.cacheRead,
          cacheWrite:   step.cacheWrite   ?? stats.cacheWrite,
          costUsd:      step.costUsd      ?? stats.costUsd,
          durationMs:   step.durationMs   ?? stats.durationMs,
          numTurns:     step.numTurns     ?? stats.numTurns,
        };
        if (steps.at(-1)?.type === 'complete') {
          return { ...j, stats };
        }
        steps = [...steps, { id: nextId, type: 'complete' }];
        nextStepId += 1;
        if (steps.length > STEP_CAP) steps = steps.slice(steps.length - STEP_CAP);
        return { ...j, steps, stats, nextStepId };
      }

      // ── Collapse repeated system messages ─────────────────
      if (step.type === 'system') {
        const last = steps[steps.length - 1];
        if (last?.type === 'system') return { ...j, stats };
      }

      // ── Phase separator — always shown, closes any open text_block ─────────
      if (step.type === 'phase') {
        const last = steps.at(-1);
        if (last?.type === 'phase' && last.text === step.text) return { ...j, stats };
        // Close any in-progress text block so the phase banner appears cleanly
        steps = [...steps, { id: nextId, type: 'phase', text: step.text }];
        nextStepId += 1;
        if (steps.length > STEP_CAP) steps = steps.slice(steps.length - STEP_CAP);
        return { ...j, steps, stats, nextStepId };
      }

      // ── Text chunk → merge into running text_block ─────────
      if (step.type === 'text_chunk') {
        const last = steps[steps.length - 1];
        if (last?.type === 'text_block') {
          const newContent = (last.content + step.text).slice(-64000);
          steps = [...steps.slice(0, -1), { ...last, content: newContent }];
          stats = { ...stats, charsGenerated: stats.charsGenerated + step.text.length };
        } else {
          steps = [...steps, { id: nextId, type: 'text_block', content: step.text.slice(-64000) }];
          nextStepId += 1;
          stats = { ...stats, charsGenerated: stats.charsGenerated + step.text.length };
        }
      }

      // ── Tool done → mark most recent active tool as done ───
      else if (step.type === 'tool_done') {
        const ri = [...steps].reverse().findIndex(s => s.type === 'tool_start' && s.status !== 'done');
        if (ri >= 0) {
          const i = steps.length - 1 - ri;
          steps = steps.map((s, idx) => idx === i ? { ...s, status: 'done' } : s);
        }
      }

      // ── Tool start → add step + increment tool counter ─────
      else if (step.type === 'tool_start') {
        steps = [...steps, { ...step, id: nextId, status: 'active' }];
        nextStepId += 1;
        const tc = { ...stats.toolCounts };
        tc[step.tool] = (tc[step.tool] || 0) + 1;
        stats = { ...stats, toolCounts: tc };
      }

      // ── Everything else ────────────────────────────────────
      else {
        steps = [...steps, { ...step, id: nextId }];
        nextStepId += 1;
      }

      if (steps.length > STEP_CAP) steps = steps.slice(steps.length - STEP_CAP);
      return { ...j, steps, stats, nextStepId };
    })
  );
}

// Merge durable records without throwing away live provider output or newer terminal events.
export function mergeHistory(records) {
  jobs.update(current => {
    const present = new Set(records.map(record => record.id));
    const byId = new Map(current.filter(job => !job.fromHistory || present.has(job.id)).map(job => [job.id, job]));
    for (const record of records) {
      if(deletedHistoryIds.has(record.id)) continue;
      const old = byId.get(record.id);
      const base = old || placeholderJob(record.id);
      const terminal = ['done','done-warnings','failed','cancelled','interrupted','blocked'];
      const staleActive = old && terminal.includes(old.status) && ['starting','working','queued'].includes(record.status);
      if (staleActive) continue;
      byId.set(record.id, {
        ...base, ...record, placeholder:false, fromHistory:true,
        status: staleActive ? old.status : record.status,
        finishedAt: staleActive ? old.finishedAt : record.finishedAt,
        steps: old?.steps || [], stats: old?.stats || emptyStats(), nextStepId: old?.nextStepId || 0,
      });
    }
    return [...byId.values()].sort((a,b) => (b.startedAt || 0) - (a.startedAt || 0));
  });
}
