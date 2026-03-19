<!-- src/App.svelte -->
<script>
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { parseLine } from './lib/parser.js';
  import {
    pendingFiles, showConfirmPanel,
    updateJobStatus, updateJobActivity, appendLogLine, selectedJobId,
  } from './stores/jobs.js';
  import Sidebar from './lib/Sidebar.svelte';
  import LogPanel from './lib/LogPanel.svelte';
  import ConfirmPanel from './lib/ConfirmPanel.svelte';

  onMount(() => {
    // New files detected by watcher
    listen('new-files', (event) => {
      $pendingFiles = event.payload;
      $showConfirmPanel = true;
    });

    // Job started (process spawned)
    listen('job-started', (event) => {
      updateJobStatus(event.payload.job_id, 'working');
    });

    // Job output line
    listen('job-output', (event) => {
      const { job_id, line } = event.payload;
      const parsed = parseLine(line);
      appendLogLine(job_id, parsed.text, parsed.tag);
      if (parsed.activity) {
        updateJobActivity(job_id, parsed.activity);
      }
    });

    // Job completed
    listen('job-done', (event) => {
      const { job_id, exit_code, output_file_exists } = event.payload;
      let status;
      if (exit_code === 0 && output_file_exists) status = 'done';
      else if (output_file_exists) status = 'done-warnings';
      else status = 'failed';
      updateJobStatus(job_id, status, Date.now());
    });
  });
</script>

<div class="app-layout">
  <div class="sidebar-container">
    <Sidebar />
    <ConfirmPanel />
  </div>
  <LogPanel />
</div>

<style>
  .app-layout {
    display: flex;
    height: 100vh;
    gap: 10px;
    padding: 12px;
    background: var(--bg-base);
  }

  .sidebar-container {
    position: relative;
    flex-shrink: 0;
  }
</style>
