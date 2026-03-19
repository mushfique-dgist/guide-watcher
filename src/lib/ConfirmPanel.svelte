<!-- src/lib/ConfirmPanel.svelte -->
<script>
  import { invoke } from '@tauri-apps/api/core';
  import { pendingFiles, showConfirmPanel, addJob, selectedJobId } from '../stores/jobs.js';

  let config = $state({ models: [], efforts: [], default_model: 'opus', default_effort: 'max' });
  let model = $state('opus');
  let effort = $state('max');
  let checked = $state([]);

  $effect(() => {
    invoke('get_config').then(c => {
      config = c;
      model = c.default_model;
      effort = c.default_effort;
    });
  });

  $effect(() => {
    // Initialize all checked when pendingFiles changes
    checked = $pendingFiles.map(() => true);
  });

  function getFilename(fp) { return fp.replace(/\\/g, '/').split('/').pop(); }
  function getFolder(fp) {
    const parts = fp.replace(/\\/g, '/').split('/');
    return parts.length > 1 ? parts[parts.length - 2] : '';
  }
  function getExt(fp) { return fp.split('.').pop().toUpperCase(); }

  async function onGenerate() {
    const selected = $pendingFiles.filter((_, i) => checked[i]);
    if (selected.length === 0) { onSkip(); return; }

    const jobIds = await invoke('approve_files', {
      filePaths: selected, model, effort,
    });

    // Create job entries in store
    for (let i = 0; i < selected.length; i++) {
      addJob({
        id: jobIds[i],
        filename: getFilename(selected[i]),
        filepath: selected[i],
        folder: getFolder(selected[i]),
        outputName: getFilename(selected[i]).replace(/\.[^.]+$/, '').replace(/ /g, '_') + '_Guide.md',
        status: 'starting',
        activity: 'Launching Claude...',
        model, effort,
        startedAt: Date.now(),
        finishedAt: null,
        logLines: [],
      });
    }

    // Auto-select first new job
    $selectedJobId = jobIds[0];
    onSkip(); // close panel
  }

  function onSkip() {
    $showConfirmPanel = false;
    $pendingFiles = [];
  }
</script>

{#if $showConfirmPanel && $pendingFiles.length > 0}
  <div class="confirm-panel">
    <h2>{$pendingFiles.length} new file{$pendingFiles.length > 1 ? 's' : ''} detected</h2>

    <div class="file-list">
      {#each $pendingFiles as fp, i (fp)}
        <label class="file-row">
          <input type="checkbox" bind:checked={checked[i]} />
          <span class="badge" class:pdf={getExt(fp) === 'PDF'} class:html={getExt(fp) === 'HTML'}>
            {getExt(fp)}
          </span>
          <span class="fname">{getFilename(fp)}</span>
          <span class="ffolder">{getFolder(fp)}</span>
        </label>
      {/each}
    </div>

    <div class="settings">
      <label>
        <span>Model</span>
        <select bind:value={model}>
          {#each config.models as m}<option value={m}>{m}</option>{/each}
        </select>
      </label>
      <label>
        <span>Effort</span>
        <select bind:value={effort}>
          {#each config.efforts as e}<option value={e}>{e}</option>{/each}
        </select>
      </label>
    </div>

    <div class="buttons">
      <button class="btn-skip" onclick={onSkip}>Skip</button>
      <button class="btn-generate" onclick={onGenerate}>Generate</button>
    </div>
  </div>
{/if}

<style>
  .confirm-panel {
    position: absolute;
    top: 0; left: 0; bottom: 0;
    width: 280px;
    background: var(--bg-base);
    border-right: 1px solid var(--border-subtle);
    border-radius: 12px 0 0 12px;
    padding: 20px 16px;
    z-index: 10;
    display: flex;
    flex-direction: column;
    animation: slideIn 0.3s ease;
  }

  h2 { font-size: 14px; font-weight: 600; margin-bottom: 16px; }

  .file-list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .file-row {
    display: flex; align-items: center; gap: 8px;
    padding: 6px 8px; border-radius: 6px; cursor: pointer;
    font-size: 11px;
  }
  .file-row:hover { background: var(--bg-surface-hover); }
  .file-row input[type="checkbox"] { accent-color: var(--accent-blue); }

  .badge {
    font-size: 8px; font-weight: 700; padding: 2px 6px; border-radius: 4px;
    text-transform: uppercase;
  }
  .badge.pdf { background: rgba(59,130,246,0.15); color: rgba(147,197,253,0.9); }
  .badge.html { background: rgba(52,211,153,0.15); color: rgba(110,231,183,0.9); }

  .fname { color: var(--text-primary); flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ffolder { color: var(--text-tertiary); font-size: 9px; }

  .settings {
    display: flex; gap: 12px; margin: 16px 0;
  }
  .settings label { display: flex; flex-direction: column; gap: 4px; flex: 1; }
  .settings label span { color: var(--text-secondary); font-size: 9px; text-transform: uppercase; letter-spacing: 1px; }
  .settings select {
    background: var(--bg-surface); color: var(--text-primary);
    border: 1px solid var(--border-subtle); border-radius: 6px;
    padding: 6px 8px; font-size: 11px;
  }

  .buttons { display: flex; gap: 8px; }
  .btn-skip {
    flex: 1; padding: 8px; border-radius: 8px;
    background: var(--bg-surface); color: var(--text-secondary);
    border: 1px solid var(--border-subtle);
    cursor: pointer; font-size: 11px; font-weight: 500;
  }
  .btn-generate {
    flex: 2; padding: 8px; border-radius: 8px;
    background: linear-gradient(135deg, var(--accent-blue), var(--accent-purple));
    color: white; border: none;
    cursor: pointer; font-size: 11px; font-weight: 600;
  }
  .btn-generate:hover { opacity: 0.9; }
</style>
