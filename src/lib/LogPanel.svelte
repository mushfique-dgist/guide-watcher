<!-- src/lib/LogPanel.svelte -->
<script>
  import { selectedJob } from '../stores/jobs.js';

  let logContainer = $state(null);
  let autoScroll = $state(true);
  let prevLineCount = $state(0);

  function formatElapsed(startedAt, finishedAt) {
    const end = finishedAt || Date.now();
    const secs = Math.floor((end - startedAt) / 1000);
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (h) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    return `${m}:${String(s).padStart(2, '0')}`;
  }

  // Tick every second
  let now = $state(Date.now());
  $effect(() => {
    const interval = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(interval);
  });

  // Auto-scroll when new lines arrive
  $effect(() => {
    const job = $selectedJob;
    if (!job || !logContainer) return;
    const count = job.logLines.length;
    if (count > prevLineCount && autoScroll) {
      logContainer.scrollTop = logContainer.scrollHeight;
    }
    prevLineCount = count;
  });

  function onScroll() {
    if (!logContainer) return;
    const { scrollTop, scrollHeight, clientHeight } = logContainer;
    autoScroll = scrollHeight - scrollTop - clientHeight < 50;
  }

  function getColorClass(tag) {
    const map = {
      read: 'log-read', bash: 'log-bash', write: 'log-write',
      text: 'log-text', dim: 'log-dim', error: 'log-error', header: 'log-tag-header',
    };
    return map[tag] || 'log-dim';
  }
</script>

{#if $selectedJob}
  {@const job = $selectedJob}
  {@const visibleLines = job.logLines.length > 500 ? job.logLines.slice(-500) : job.logLines}

  <div class="log-panel">
    <!-- Header -->
    <div class="log-header-bar">
      <div class="log-title-area">
        <div class="log-filename">{job.filename}</div>
        <div class="log-meta">{job.folder} · {job.model} · {job.effort} effort</div>
      </div>
      <div class="log-timer">
        <span>{formatElapsed(job.startedAt, job.finishedAt)}</span>
      </div>
    </div>

    <!-- Log lines -->
    <div class="log-body" bind:this={logContainer} onscroll={onScroll}>
      {#each visibleLines as line (line.lineNum)}
        <div class="log-line">
          <span class="line-num">{String(line.lineNum).padStart(3, ' ')}</span>
          <span class={getColorClass(line.tag)}>{line.text}</span>
        </div>
      {/each}
      {#if job.status === 'working' || job.status === 'starting'}
        <div class="log-line">
          <span class="line-num">&nbsp;</span>
          <span class="cursor">█</span>
        </div>
      {/if}
    </div>

    <!-- Status bar -->
    <div class="log-statusbar">
      <div class="status-left">
        {#if job.status === 'working' && job.activity}
          <span class="dot-mini active"></span>
          <span>{job.activity}</span>
        {:else if job.status === 'done'}
          <span class="dot-mini done"></span>
          <span>Complete</span>
        {:else if job.status === 'failed'}
          <span class="dot-mini failed"></span>
          <span>Failed</span>
        {:else}
          <span class="dot-mini active"></span>
          <span>Starting...</span>
        {/if}
      </div>
      <span class="event-count">{job.logLines.length} events</span>
    </div>
  </div>
{:else}
  <div class="log-panel empty">
    <span>Click a job to view its output</span>
  </div>
{/if}

<style>
  .log-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
    border-radius: 12px;
    border: 1px solid var(--border-panel);
    overflow: hidden;
  }
  .log-panel.empty {
    justify-content: center;
    align-items: center;
    color: var(--text-tertiary);
    font-size: 12px;
  }

  .log-header-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 16px;
    border-bottom: 1px solid rgba(255,255,255,0.05);
  }
  .log-filename { font-size: 12px; font-weight: 600; }
  .log-meta { color: var(--text-tertiary); font-size: 9px; margin-top: 3px; }
  .log-timer {
    background: rgba(59,130,246,0.08);
    padding: 4px 12px;
    border-radius: 8px;
    border: 1px solid rgba(59,130,246,0.12);
    color: rgba(147,197,253,0.9);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .log-body {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px;
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 2;
  }

  .log-line { display: flex; white-space: nowrap; }
  .line-num {
    color: var(--log-line-num);
    width: 32px;
    text-align: right;
    margin-right: 12px;
    user-select: none;
    font-size: 10px;
    flex-shrink: 0;
  }

  .log-read { color: var(--log-read); }
  .log-bash { color: var(--log-bash); }
  .log-write { color: var(--log-write); }
  .log-text { color: var(--log-text); }
  .log-dim { color: var(--log-dim); }
  .log-error { color: var(--accent-red); }
  .log-tag-header { color: var(--accent-purple); font-weight: bold; }

  .cursor { color: rgba(255,255,255,0.1); animation: pulse 1s infinite; }

  .log-statusbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 16px;
    border-top: 1px solid rgba(255,255,255,0.04);
    font-size: 9px;
    color: var(--text-tertiary);
  }
  .status-left { display: flex; align-items: center; gap: 6px; }
  .event-count { color: var(--text-dim); }

  .dot-mini { width: 5px; height: 5px; border-radius: 50%; }
  .dot-mini.active { background: var(--accent-blue); box-shadow: 0 0 6px rgba(59,130,246,0.4); }
  .dot-mini.done { background: var(--accent-green); }
  .dot-mini.failed { background: var(--accent-red); }
</style>
