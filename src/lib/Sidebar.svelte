<!-- src/lib/Sidebar.svelte -->
<script>
  import { selectedJobId, activeJobs, completedJobs } from '../stores/jobs.js';

  function formatElapsed(startedAt, finishedAt) {
    const end = finishedAt || Date.now();
    const secs = Math.floor((end - startedAt) / 1000);
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (h) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    return `${m}:${String(s).padStart(2, '0')}`;
  }

  function selectJob(id) {
    $selectedJobId = id;
  }

  // Tick every second for elapsed time
  let now = $state(Date.now());
  $effect(() => {
    const interval = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(interval);
  });
</script>

<aside class="sidebar">
  <!-- Watching status -->
  <div class="watch-status">
    <span class="dot watching"></span>
    <span class="watch-label">Watching</span>
  </div>

  <!-- Active section -->
  {#if $activeJobs.length > 0}
    <div class="section-label">ACTIVE</div>
    {#each $activeJobs as job (job.id)}
      <button
        class="job-item"
        class:selected={$selectedJobId === job.id}
        onclick={() => selectJob(job.id)}
      >
        <div class="job-header">
          <span class="dot active"></span>
          <span class="job-name">{job.filename}</span>
        </div>
        <div class="job-meta">
          <span class="job-folder">{job.folder}</span>
          <span class="job-time">{formatElapsed(job.startedAt, null)}</span>
        </div>
        {#if job.activity}
          <div class="job-activity">{job.activity}</div>
        {/if}
      </button>
    {/each}
  {/if}

  <!-- Completed section -->
  {#if $completedJobs.length > 0}
    <div class="section-label">COMPLETED</div>
    {#each $completedJobs as job (job.id)}
      <button
        class="job-item"
        class:selected={$selectedJobId === job.id}
        onclick={() => selectJob(job.id)}
      >
        <div class="job-header">
          <span class="dot"
            class:done={job.status === 'done'}
            class:warnings={job.status === 'done-warnings'}
            class:failed={job.status === 'failed'}
          ></span>
          <span class="job-name dim">{job.filename}</span>
        </div>
        <div class="job-meta">
          <span class="job-folder">{job.folder}</span>
          <span class="job-time">{formatElapsed(job.startedAt, job.finishedAt)}</span>
        </div>
      </button>
    {/each}
  {/if}

  {#if $activeJobs.length === 0 && $completedJobs.length === 0}
    <div class="empty-state">No jobs yet</div>
  {/if}
</aside>

<style>
  .sidebar {
    width: 240px;
    height: 100%;
    background: var(--glass-bg, linear-gradient(180deg, rgba(255,255,255,0.04), rgba(255,255,255,0.015)));
    border-right: 1px solid var(--border-subtle);
    border-radius: 12px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
  }

  .watch-status {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    background: var(--bg-surface);
    border-radius: 8px;
    margin-bottom: 12px;
  }
  .watch-label { color: var(--text-secondary); font-size: 11px; font-weight: 500; }

  .section-label {
    color: var(--text-tertiary);
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 1.8px;
    font-weight: 600;
    padding: 10px 4px 6px;
  }

  .job-item {
    all: unset;
    cursor: pointer;
    display: block;
    width: 100%;
    padding: 10px 12px;
    border-radius: 10px;
    transition: background 0.15s;
  }
  .job-item:hover { background: var(--bg-surface-hover); }
  .job-item.selected {
    background: var(--bg-active);
    border: 1px solid var(--border-active);
  }

  .job-header { display: flex; align-items: center; gap: 8px; }
  .job-name { color: var(--text-primary); font-size: 11px; font-weight: 600; }
  .job-name.dim { color: var(--text-secondary); font-weight: 400; }

  .job-meta {
    display: flex; justify-content: space-between;
    margin-top: 4px; padding-left: 18px;
  }
  .job-folder { color: var(--text-tertiary); font-size: 9px; }
  .job-time { color: rgba(147,197,253,0.6); font-size: 9px; font-variant-numeric: tabular-nums; }

  .job-activity {
    color: rgba(147,197,253,0.5);
    font-size: 8px;
    margin-top: 4px;
    padding-left: 18px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Dots */
  .dot { width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0; }
  .dot.watching { background: var(--accent-green); box-shadow: 0 0 8px rgba(52,211,153,0.4); }
  .dot.active { background: var(--accent-blue); box-shadow: 0 0 8px rgba(59,130,246,0.4); animation: pulse 1.5s ease-in-out infinite; }
  .dot.done { background: var(--accent-green); }
  .dot.warnings { background: var(--accent-yellow); }
  .dot.failed { background: var(--accent-red); }

  .empty-state {
    color: var(--text-tertiary);
    font-size: 11px;
    text-align: center;
    padding: 24px 0;
  }
</style>
