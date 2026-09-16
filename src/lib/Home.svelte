<script>
import { recentJobs, activeJobs, pendingFiles, showConfirmPanel, selectedJobId } from '../stores/jobs.js';
import { currentView,busyAction,historyReturnView } from '../stores/navigation.js';
import { chooseSources } from './sourceActions.js';
import { statusLabel,dateTime } from './ui.js';
import { powerMode } from '../stores/preferences.js';
</script>
<section class="page" aria-labelledby="home-title"><div class="page-inner">
<div class="page-heading"><div><div class="eyebrow">Your study workspace</div><h1 id="home-title">Turn course material into understanding.</h1><p>Build detailed guides from your lectures, with context from your books and earlier guides.</p></div></div>
<div class="start-section"><h2>Start a guide</h2><p class="muted">Choose the lecture files, or let the app find new material in your semester folder.</p><div class="actions"><button class="button primary" onclick={()=>chooseSources('pick')} disabled={!!$busyAction}>{$busyAction==='pick'?'Opening files…':'Choose lecture files'}</button><button class="button secondary" onclick={()=>chooseSources('scan')} disabled={!!$busyAction}>{$busyAction==='scan'?'Scanning semester…':'Scan semester'}</button>{#if $powerMode}<button class="button ghost" onclick={()=>chooseSources('resume')} disabled={!!$busyAction}>{$busyAction==='resume'?'Opening saved prep…':'Use saved prep'}</button>{/if}</div><small class="muted">PDF, PowerPoint, Word, and HTML. Your original files are never changed.</small></div>
{#if $pendingFiles.length}<div class="pending"><div><strong>{$pendingFiles.length} sources ready to review</strong><p class="muted">Check your selection before generation begins.</p></div><button class="button secondary" onclick={()=>{$showConfirmPanel=true;}}>Review sources</button></div>{/if}
<div class="workflow"><h2>From lecture to guide</h2>{#if $powerMode}<ol><li><strong>Gather context</strong><span>Codex reads the material and prepares the evidence.</span></li><li><strong>Write and explain</strong><span>Claude develops the guide with worked explanations and visuals.</span></li><li><strong>Check and save</strong><span>The app validates the output before marking it successful.</span></li></ol>{:else}<ol><li><strong>Reads your material</strong><span>The lecture, your textbooks, and the guides you already have.</span></li><li><strong>Writes the guide</strong><span>Worked examples, figures from your slides, and exam-style practice.</span></li><li><strong>Checks it</strong><span>Nothing is saved as finished until it passes the app's own checks.</span></li></ol>{/if}</div>
<div class="recent"><div class="section-heading"><h2>Recent activity</h2><button class="button ghost" onclick={()=>{$currentView='history';}}>View history →</button></div>
{#each $recentJobs.slice(0,5) as job (job.id)}<button class="recent-row" onclick={()=>{$historyReturnView='history';$selectedJobId=job.id;$currentView='run';}}><span><strong>{job.filename}</strong><small>{job.folder} · {dateTime(job.startedAt)}</small></span><span class="status-badge {job.status}">{statusLabel(job.status)}</span></button>{:else}<p class="empty-history">No recorded runs yet. Your results and recovery options will stay in History after you close the app.</p>{/each}
</div></div></section>
<style>
#home-title {margin-top:.55rem;max-width:24ch;font-size:2rem;}
.start-section {padding:1.5rem 0;border-top:1px solid var(--border-panel);border-bottom:1px solid var(--border-panel);display:grid;gap:.65rem;}
.actions {display:flex;gap:.65rem;flex-wrap:wrap;margin:.45rem 0;}
.pending {display:flex;flex-wrap:wrap;gap:1rem;justify-content:space-between;align-items:center;padding:1rem;background:var(--bg-active);border-radius:8px;margin-top:1.5rem;}
.workflow {padding:2rem 0;}
ol {list-style:none;counter-reset:step;padding:0;margin:1rem 0 0;display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:1.5rem;}
li {counter-increment:step;display:flex;flex-direction:column;gap:.45rem;font-size:.95rem;}
li::before {content:'0' counter(step);font-variant-numeric:tabular-nums;color:var(--text-tertiary);font-size:.85rem;}
li span {color:var(--text-secondary);line-height:1.6;}
.recent {border-top:1px solid var(--border-panel);padding-top:1rem;}
.section-heading {display:flex;justify-content:space-between;align-items:center;gap:1rem;flex-wrap:wrap;}
.recent-row {width:100%;display:flex;justify-content:space-between;align-items:center;gap:1rem;padding:1rem 0;text-align:left;border-bottom:1px solid var(--border-panel);}
.recent-row>span:first-child {min-width:0;display:grid;gap:.3rem;overflow-wrap:anywhere;}
.recent-row strong {font-weight:500;}
.recent-row small {color:var(--text-secondary);}
.recent-row:hover {background:var(--bg-surface);}
.empty-history {color:var(--text-secondary);padding:1rem 0;max-width:60ch;}
@container(max-width:750px) {ol {grid-template-columns:1fr;gap:1rem;}li {gap:.2rem;}#home-title{font-size:1.7rem;}.recent-row {align-items:flex-start;flex-direction:column;gap:.5rem;}}
</style>