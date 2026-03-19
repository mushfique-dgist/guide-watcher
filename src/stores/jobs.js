import { writable, derived } from 'svelte/store';

export const jobs = writable([]);
export const selectedJobId = writable(null);
export const pendingFiles = writable([]);
export const showConfirmPanel = writable(false);

const LOG_CAP = 2000;

export const selectedJob = derived(
  [jobs, selectedJobId],
  ([$jobs, $selectedJobId]) => $jobs.find(j => j.id === $selectedJobId) ?? null
);

export const activeJobs = derived(jobs, $jobs =>
  $jobs.filter(j => j.status === 'starting' || j.status === 'working')
);

export const completedJobs = derived(jobs, $jobs =>
  $jobs.filter(j => j.status === 'done' || j.status === 'done-warnings' || j.status === 'failed')
);

export function addJob(job) {
  jobs.update(list => [job, ...list]);
}

export function updateJobStatus(jobId, status, finishedAt = null) {
  jobs.update(list =>
    list.map(j => j.id === jobId ? { ...j, status, finishedAt } : j)
  );
}

export function updateJobActivity(jobId, activity) {
  jobs.update(list =>
    list.map(j => j.id === jobId ? { ...j, activity } : j)
  );
}

export function appendLogLine(jobId, text, tag) {
  jobs.update(list =>
    list.map(j => {
      if (j.id !== jobId) return j;
      const lineNum = j.logLines.length + 1;
      let logLines = [...j.logLines, { text, tag, lineNum }];
      // Cap at LOG_CAP, evict oldest
      if (logLines.length > LOG_CAP) {
        logLines = logLines.slice(logLines.length - LOG_CAP);
      }
      return { ...j, logLines };
    })
  );
}
