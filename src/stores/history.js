import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { mergeHistory, forgetHistoryJob } from './jobs.js';
import { filename,folder } from '../lib/ui.js';
export const historyLoading=writable(false);
export const historyError=writable('');
export const historyMutation=writable('');
let inFlight=null;
function modelSettings(options) {
  if(!options || typeof options!=='object')return {};
  const prep=options.prep;
  const writer=options.writer;
  const collections=Array.isArray(options.collectionFallbacks)?options.collectionFallbacks:[];
  const fallbacks=Array.isArray(options.fallbacks)?options.fallbacks:[];
  const validPhase=phase=>phase&&typeof phase.model==='string'&&phase.model.trim()&&typeof phase.effort==='string'&&phase.effort.trim();
  return {
    ...(validPhase(prep)?{prepProvider:prep.provider||'',prepModel:prep.model,prepEffort:prep.effort}:{}),
    ...(validPhase(writer)?{provider:writer.provider||'',model:writer.model,effort:writer.effort}:{}),
    collectionFallbackChain:collections.filter(validPhase).map(({model,effort})=>({model,effort})),
    fallbackChain:fallbacks.filter(validPhase).map(({model,effort})=>({model,effort})),
  };
}
export function normalizeHistoryRecord(record) {
  const statuses=['queued','starting','working','done','done-warnings','failed','cancelled','interrupted','blocked'];
  if(!record || typeof record.jobId!=='string' || !record.jobId.trim() || !statuses.includes(record.status)) throw new Error('A saved history record has an invalid identity or outcome.');
  if(record.events!=null && (!Array.isArray(record.events) || record.events.some(event=>!event || typeof event.message!=='string'))) throw new Error('A saved history record has invalid activity entries.');

  if(record.archivedAt!=null && (!Number.isFinite(record.archivedAt) || record.archivedAt<=0)) throw new Error('A saved history record has an invalid archive date.');
  return {...record,...modelSettings(record.options),archivedAt:record.archivedAt??null,id:record.jobId,filename:record.filename||filename(record.sourcePath),filepath:record.sourcePath||'',folder:record.folder||folder(record.sourcePath),outputPath:record.outputPath,prepPath:record.prepPath,status:record.status,startedAt:record.startedAt,finishedAt:record.finishedAt,summary:record.summary||'',nextStep:typeof record.nextStep==='string'?record.nextStep:null,events:record.events||[],canResume:record.canResume===true,canRetry:record.canRetry===true,canOpenOutput:record.canOpenOutput===true};
}
export function refreshHistory(){
  if(inFlight)return inFlight;
  historyLoading.set(true);
  inFlight=invoke('list_job_history').then(records=>{
    if(!Array.isArray(records))throw new Error('The app returned an invalid history response.');
    const normalized=records.map(normalizeHistoryRecord);
    if(new Set(normalized.map(record=>record.id)).size!==normalized.length)throw new Error('The app returned duplicate history identities.');
    mergeHistory(normalized);historyError.set('');return true;
  }).catch(error=>{historyError.set(String(error));return false;}).finally(()=>{historyLoading.set(false);inFlight=null;});
  return inFlight;
}
export async function runHistoryAction(action,jobId,options=null){
  const command=({resume:'resume_history_job',retry:'retry_history_job',cancel:'cancel_history_job',open:'open_history_output'})[action];
  if(!command)throw new Error('Unknown recovery action');
  // Resume carries the user's current generation preferences so the writer model can change
  // between attempts (for example after a Claude quota or login failure).
  const payload=action==='resume'&&options?{jobId,options}:{jobId};
  const result=await invoke(command,payload);await refreshHistory();return result;
}

// Serialize management actions and refresh after any older in-flight snapshot settles.
export async function manageHistory(action,jobId) {
  const command=({archive:'archive_history_job',clear:'clear_recent_history',restore:'restore_history_job',delete:'delete_history_job'})[action];
  if(!command)throw new Error('Unknown history action.');
  if(action!=='clear' && (typeof jobId!=='string'||!jobId.trim()))throw new Error('Choose a history record first.');
  if(get(historyMutation))throw new Error('A history change is already being saved.');
  historyMutation.set(action);
  try {
    await invoke(command,action==='clear'?{}:{jobId});
    if(action==='delete')forgetHistoryJob(jobId);
    if(inFlight)await inFlight;
    if(!await refreshHistory())throw new Error('The history change was saved, but the updated list could not be loaded. Refresh to see it.');
  } finally {historyMutation.set('');}
}
