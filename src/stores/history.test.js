import test from 'node:test';
import assert from 'node:assert/strict';
import { get } from 'svelte/store';
import { jobs,recentJobs,activeJobs } from './jobs.js';
import { normalizeHistoryRecord,refreshHistory,manageHistory,historyMutation,historyError } from './history.js';
const record=(id='a',extra={})=>({jobId:id,status:'interrupted',sourcePath:'C:/course/source.pdf',startedAt:1,events:[],...extra});
function backend(invoke){globalThis.window={__TAURI_INTERNALS__:{invoke}};jobs.set([]);historyMutation.set('');historyError.set('');}
test('legacy records default to visible and malformed archive dates fail validation',()=>{
 assert.equal(normalizeHistoryRecord(record()).archivedAt,null);
 for(const value of [0,-1,'12',false,NaN,Infinity])assert.throws(()=>normalizeHistoryRecord(record('a',{archivedAt:value})),/archive date/);
 assert.equal(normalizeHistoryRecord(record('a',{archivedAt:12})).archivedAt,12);
});
test('archive hides recent record while full history and active work remain',async()=>{
 const rows=[record(),record('active',{status:'working'})];
 backend(async command=>{if(command==='list_job_history')return structuredClone(rows);assert.equal(command,'archive_history_job');rows[0].archivedAt=10;});
 await refreshHistory();await manageHistory('archive','a');
 assert.equal(get(jobs).length,2);assert.deepEqual(get(recentJobs).map(r=>r.id),['active']);assert.equal(get(activeJobs).length,1);
});
test('duplicate management actions are blocked while save is pending',async()=>{
 let release;let count=0;
 backend(async command=>{if(command==='list_job_history')return [];count++;await new Promise(r=>release=r);});
 const action=manageHistory('clear');await assert.rejects(manageHistory('clear'),/already being saved/);
 assert.equal(count,1);release();await action;assert.equal(get(historyMutation),'');
});
test('failed save keeps history visible and permits retry',async()=>{
 let fail=true;
 backend(async command=>{if(command==='list_job_history')return [record()];if(fail)throw Error('Storage unavailable');return;});
 await refreshHistory();await assert.rejects(manageHistory('archive','a'),/Storage unavailable/);assert.equal(get(recentJobs).length,1);assert.equal(get(historyMutation),'');
 fail=false;await manageHistory('archive','a');
});
test('an older snapshot cannot undo a completed management action',async()=>{
 let release;let reads=0;let archived=false;
 backend(async command=>{if(command==='archive_history_job'){archived=true;return;}reads++;if(reads===1)return [record()];if(reads===2){const snapshot=[record()];await new Promise(r=>release=r);return snapshot;}return [record('a',{archivedAt:archived?10:null})];});
 await refreshHistory();const stale=refreshHistory();const action=manageHistory('archive','a');await Promise.resolve();release();await stale;await action;
 assert.equal(reads,3);assert.equal(get(recentJobs).length,0);assert.equal(get(jobs)[0].archivedAt,10);
});
test('delete removes durable record from all lists after authoritative refresh',async()=>{
 let removed=false;
 backend(async command=>{if(command==='list_job_history')return removed?[]:[record('to-delete',{archivedAt:10})];assert.equal(command,'delete_history_job');removed=true;});
 await refreshHistory();await manageHistory('delete','to-delete');assert.equal(get(jobs).length,0);assert.equal(get(recentJobs).length,0);
});
test('successful mutation followed by failed refresh reports saved state honestly',async()=>{
 let changed=false;
 backend(async command=>{if(command==='list_job_history'){if(changed)throw Error('Read failed');return [record()];}changed=true;});
 await refreshHistory();await assert.rejects(manageHistory('archive','a'),/was saved.*could not be loaded/);assert.equal(get(historyMutation),'');assert.equal(get(jobs).length,1);
});
test('unknown commands and empty identities never invoke the backend',async()=>{
 let count=0;backend(async()=>count++);
 await assert.rejects(manageHistory('erase','a'),/Unknown/);
 for(const id of [undefined,null,'',' '])await assert.rejects(manageHistory('archive',id),/Choose/);
 assert.equal(count,0);
});

test('history maps identities and paths without trusting truthy capability values',()=>{
 const row=normalizeHistoryRecord(record('mapped',{sourcePath:'C:/course/source.pdf',canResume:'true',canRetry:1,canOpenOutput:true}));
 assert.equal(row.id,'mapped');assert.equal(row.filepath,'C:/course/source.pdf');assert.equal(row.filename,'source.pdf');assert.equal(row.canResume,false);assert.equal(row.canRetry,false);assert.equal(row.canOpenOutput,true);
});
test('history restores the exact collection and writing model flow',()=>{
 const options={prep:{provider:'codex-chatgpt',model:'gpt-5.6-luna',effort:'medium'},collectionFallbacks:[{provider:'codex-chatgpt',model:'gpt-5.6-terra',effort:'medium'},{provider:'codex-chatgpt',model:'gpt-5.6-sol',effort:'medium'}],writer:{provider:'claude-code',model:'claude-opus-4-8',effort:'high'},fallbacks:[{provider:'codex-chatgpt',model:'gpt-5.6-sol',effort:'medium'}]};
 const row=normalizeHistoryRecord(record('models',{options}));
 assert.equal(row.prepModel,'gpt-5.6-luna');assert.equal(row.prepEffort,'medium');
 assert.deepEqual(row.collectionFallbackChain,[{model:'gpt-5.6-terra',effort:'medium'},{model:'gpt-5.6-sol',effort:'medium'}]);
 assert.equal(row.model,'claude-opus-4-8');assert.equal(row.effort,'high');
 assert.deepEqual(row.fallbackChain,[{model:'gpt-5.6-sol',effort:'medium'}]);
});
test('history rejects malformed identities, statuses and activity',()=>{
 for(const r of [null,{},record(''),record(' '),record('a',{status:'success'}),record('a',{events:{}}),record('a',{events:[null]}),record('a',{events:[{message:12}]})])assert.throws(()=>normalizeHistoryRecord(r));
});

test('next-step guidance and event detail survive normalization',()=>{const r=normalizeHistoryRecord(record('a',{nextStep:'Click Resume writing.',events:[{timestamp:1,level:'error',message:'The guide did not pass checks.',detail:'[FAIL] X',nextStep:'Resume writing'}]}));assert.equal(r.nextStep,'Click Resume writing.');assert.equal(r.events[0].detail,'[FAIL] X');assert.equal(r.events[0].nextStep,'Resume writing');assert.equal(normalizeHistoryRecord(record('b')).nextStep,null);});

test('late provider events and stale history cannot resurrect a deleted record',async()=>{
 const {forgetHistoryJob,appendStep,updateJobStatus,mergeHistory,addJob}=await import('./jobs.js');
 jobs.set([]);forgetHistoryJob('deleted-late');updateJobStatus('deleted-late','working');appendStep('deleted-late',{type:'text_chunk',text:'late'});addJob({id:'deleted-late',status:'starting'});mergeHistory([normalizeHistoryRecord(record('deleted-late'))]);assert.equal(get(jobs).length,0);
});

test('resume forwards the current generation preferences and other actions send only the id',async()=>{
  const {runHistoryAction}=await import('./history.js');
  const calls=[];
  backend(async(command,args)=>{calls.push([command,args]);if(command==='list_history')return [];return {jobId:'next'};});
  const options={prep:{provider:'codex-chatgpt',model:'gpt-5.6-luna',effort:'medium'},collectionFallbacks:[],writer:{provider:'claude-code',model:'claude-sonnet-5',effort:'high'},fallbacks:[]};
  await runHistoryAction('resume','job-1',options);
  assert.deepEqual(calls[0],['resume_history_job',{jobId:'job-1',options}]);
  await runHistoryAction('resume','job-2');
  assert.deepEqual(calls.find(([c,a])=>a?.jobId==='job-2'),['resume_history_job',{jobId:'job-2'}]);
  await runHistoryAction('open','job-3',options);
  assert.deepEqual(calls.find(([c,a])=>a?.jobId==='job-3'),['open_history_output',{jobId:'job-3'}]);
});
