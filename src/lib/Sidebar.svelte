<script>
  import { selectedJobId, activeJobs, recentJobs, pendingFiles, showConfirmPanel } from '../stores/jobs.js';
  import { currentView, busyAction, actionError, actionNotice, historyReturnView } from '../stores/navigation.js';
  import { chooseSources } from './sourceActions.js';
  import { statusLabel } from './ui.js';
  import { manageHistory, historyMutation } from '../stores/history.js';
  import { isActive } from './ui.js';
  const recent=$derived($recentJobs.filter(j=>!isActive(j.status)));
  async function clearRecent(id) {try {await manageHistory(id?'archive':'clear',id);$actionNotice=id?'Removed from Recents. Find it in Settings → Full history.':'Recents cleared. Your records are kept in Settings → Full history.';}catch(e){$actionError=String(e);}}
  let { onnavigate = () => {} } = $props();
  function navigate(view) { $showConfirmPanel=false; $currentView=view; onnavigate(); }
  function selectJob(id) { $historyReturnView='history'; $selectedJobId=id; navigate('run'); }
</script>
<aside class="sidebar" aria-label="Main navigation">
  <button class="brand" onclick={() => navigate('home')} aria-label="Guide Watcher home"><span class="brand-mark" aria-hidden="true">g.</span><span>Guide Watcher</span></button>
  <button class="button primary new-guide" onclick={() => {navigate('home'); chooseSources('pick');}} disabled={!!$busyAction}><span aria-hidden="true">+</span> New guide</button>
  <nav aria-label="Workspace">
    <button class:chosen={$currentView==='home' && !$showConfirmPanel} aria-current={$currentView==='home' && !$showConfirmPanel ? 'page' : undefined} onclick={() => navigate('home')}><span aria-hidden="true">▤</span> Workspace</button>
    <button class:chosen={$currentView==='history' && !$showConfirmPanel} aria-current={$currentView==='history' && !$showConfirmPanel ? 'page' : undefined} onclick={() => navigate('history')}><span aria-hidden="true">◷</span> History <span class="count">{$recentJobs.length}</span></button>
    {#if $pendingFiles.length}<button class:chosen={$showConfirmPanel} onclick={() => {$showConfirmPanel=true;onnavigate();}}><span aria-hidden="true">☑</span> Review sources <span class="count">{$pendingFiles.length}</span></button>{/if}
  </nav>
  <div class="run-list">
    <div class="list-heading"><span class="eyebrow">In progress</span><span class="count">{$activeJobs.length}</span></div>
    {#each $activeJobs as job (job.id)}
      <button class="run" class:selected={$selectedJobId===job.id && $currentView==='run'} onclick={() => selectJob(job.id)} title={job.filepath}>
        <span class="run-title">{job.filename}</span><span class="run-status">{statusLabel(job.status)} · {job.folder || 'Preparing sources'}</span>
      </button>
    {:else}<p class="quiet">Your active guides will appear here.</p>{/each}
    {#if recent.length}
      <div class="list-heading recent-heading"><span class="eyebrow">Recent</span><button class="clear-recents" disabled={!!$historyMutation} onclick={()=>clearRecent()} title="Keep records in Settings → Full history">Clear recents</button></div>
      {#each recent.slice(0,6) as job (job.id)}
        <div class="recent-item"><button class="run" class:selected={$selectedJobId===job.id && $currentView==='run'} onclick={() => selectJob(job.id)} title={job.filepath}><span class="run-title">{job.filename}</span><span class="run-status">{statusLabel(job.status)}</span></button><button class="remove-recent" aria-label={'Remove '+job.filename+' from Recents'} title="Remove from Recents; keep in Full history" disabled={!!$historyMutation} onclick={()=>clearRecent(job.id)}>×</button></div>
      {/each}
    {/if}
  </div>
  <div class="sidebar-bottom"><button class="settings-link" class:chosen={['settings','full-history'].includes($currentView)} onclick={() => navigate('settings')}><span aria-hidden="true">⚙</span> Settings & help</button><span class="local-note">Saved on this computer</span></div>
</aside>
<style>
.sidebar { height:100%;min-height:0;display:flex;flex-direction:column;gap:1rem;padding:1rem .85rem;background:var(--bg-sidebar);overflow:hidden; }
.brand {display:flex;align-items:center;gap:.65rem;text-align:left;padding:.35rem;font-weight:600;font-size:1rem;}
.brand-mark {font-size:1.55rem;line-height:1;font-weight:650;color:var(--accent-green);letter-spacing:-.09em;margin-right:.2rem;}
.new-guide {width:100%;justify-content:flex-start;}
nav {display:grid;gap:.3rem;}
nav button,.settings-link {display:flex;align-items:center;gap:.7rem;padding:.65rem .7rem;text-align:left;border-radius:6px;font-size:.95rem;color:var(--text-secondary);min-width:0;}
nav button:hover,.settings-link:hover {background:var(--bg-surface);color:var(--text-primary);}
nav button.chosen,.settings-link.chosen {background:var(--bg-surface-hover);color:var(--text-primary);}
.count {margin-left:auto;font-size:.8rem;color:var(--text-secondary);font-variant-numeric:tabular-nums;}
.run-list {flex:1;min-height:0;overflow-y:auto;overflow-x:hidden;scrollbar-gutter:stable;padding:.4rem;scroll-padding:.4rem;}
.brand,.new-guide,nav,.sidebar-bottom{flex-shrink:0;}
.recent-item{display:flex;align-items:center;min-width:0;}.recent-item .run{flex:1;min-width:0;}.remove-recent{flex:none;min-width:2rem;min-height:2rem;border-radius:5px;color:var(--text-secondary);}.remove-recent:hover,.clear-recents:hover{background:var(--bg-surface-hover);color:var(--text-primary);}.clear-recents{margin-left:auto;font-size:.75rem;min-height:2rem;padding:.2rem;border-radius:4px;}.recent-heading{gap:.3rem;}
@container(max-height:480px){.sidebar{overflow-y:auto;}.run-list{flex:none;overflow:visible;}.sidebar-bottom{margin-top:auto;}}
.list-heading {display:flex;align-items:center;padding:.4rem .7rem;}
.recent-heading {margin-top:1rem;}
.quiet {color:var(--text-tertiary);font-size:.85rem;padding:.5rem .7rem;line-height:1.6;}
.run {width:100%;display:flex;flex-direction:column;gap:.25rem;text-align:left;padding:.7rem;border-radius:6px;min-width:0;}
.run:hover {background:var(--bg-surface);}
.run.selected {background:var(--bg-active);}
.run-title {font-size:.9rem;max-width:100%;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;}
.run-status {font-size:.8rem;color:var(--text-secondary);max-width:100%;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;}
.sidebar-bottom {background:var(--bg-sidebar);border-top:1px solid var(--border-panel);padding-top:.5rem;display:grid;gap:.5rem;}
.local-note {font-size:.75rem;color:var(--text-tertiary);padding:0 .7rem;}
</style>