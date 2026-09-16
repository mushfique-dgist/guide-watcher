import test,{beforeEach} from 'node:test';
import assert from 'node:assert/strict';
import {get} from 'svelte/store';
import {confirmationConfig,confirmationError,confirmationBusy,sourceChoices,selectedCourse,prepModel,prepEffort,collectionFallback1Model,collectionFallback1Effort,collectionFallback2Model,collectionFallback2Effort,writerModel,writerEffort,claudeFallbackEnabled,claudeFallbackModel,claudeFallbackEffort,codexFallbackModel,codexFallbackEffort,GENERATION_PREFERENCES_KEY,isChosen,setChosen,initializeGenerationSelections,normalizeGenerationPreferences,saveGenerationPreferences,resetGenerationPreferences,generationSelectionSnapshot,loadConfirmationConfig,submitConfirmation,QUALITY_PRESETS,availableQualityPresets,applyQualityPreset,activeQualityPreset} from './confirmation.js';
import {pendingFiles,pendingSource,showConfirmPanel,jobs,selectedJobId,receiveWatcherFiles,deferredWatcherFiles} from '../stores/jobs.js';
import {busyAction,currentView} from '../stores/navigation.js';
const config={providers:[{id:'codex-chatgpt',models:['gpt-5.6-sol','gpt-5.6-terra','gpt-5.6-luna'],efforts:['low','medium','high','xhigh','max']},{id:'claude-code',models:['claude-opus-4-8','claude-opus-5'],efforts:['low','medium','high','xhigh','max']}],default_prep:{provider:'codex-chatgpt',model:'gpt-5.6-luna',effort:'medium'},collection_fallback_chain:[{provider:'codex-chatgpt',model:'gpt-5.6-terra',effort:'medium'},{provider:'codex-chatgpt',model:'gpt-5.6-sol',effort:'medium'}],default_writer:{provider:'claude-code',model:'claude-opus-4-8',effort:'high'},fallback_chain:[{provider:'codex-chatgpt',model:'gpt-5.6-sol',effort:'medium'}],course_profiles:[]};
let calls=[];let handler;let savedPreferences;
beforeEach(()=>{calls=[];savedPreferences=new Map();globalThis.localStorage={getItem:key=>savedPreferences.get(key)??null,setItem:(key,value)=>savedPreferences.set(key,String(value)),removeItem:key=>savedPreferences.delete(key)};handler=async(command)=>command==='get_config'?config:[];globalThis.window={__TAURI_INTERNALS__:{invoke:async(command,args)=>{calls.push({command,args});return handler(command,args);}}};initializeGenerationSelections(config,{force:true});confirmationConfig.set(config);confirmationError.set('');confirmationBusy.set(false);sourceChoices.set({});selectedCourse.set('auto');pendingFiles.set([]);pendingSource.set('watcher');showConfirmPanel.set(true);jobs.set([]);selectedJobId.set(null);deferredWatcherFiles.set([]);busyAction.set('');});
test('selection identity survives path case and separator aliases',()=>{setChosen(['C:/Course/Lecture.pdf'],false);assert.equal(isChosen('c:'+String.fromCharCode(92)+'course'+String.fromCharCode(92)+'lecture.PDF'),false);assert.equal(isChosen('C:/Course/Next.pdf'),true);});
test('missing configuration and empty selection never submit',async()=>{confirmationConfig.set(null);await submitConfirmation();assert.equal(calls.length,0);confirmationConfig.set(config);await submitConfirmation();assert.equal(calls.length,0);assert.match(get(confirmationError),/Select at least/);});
test('submission survives navigation and keeps unchecked and newly arrived files',async()=>{pendingFiles.set(['C:/Course/one.pdf','C:/Course/two.pdf']);setChosen(['C:/Course/two.pdf'],false);let resolve;handler=command=>command==='approve_files'?new Promise(r=>resolve=r):Promise.resolve([]);const first=submitConfirmation();assert.equal(get(confirmationBusy),true);showConfirmPanel.set(false);currentView.set('history');await submitConfirmation();assert.equal(calls.filter(c=>c.command==='approve_files').length,1);receiveWatcherFiles(['C:/Course/three.pdf']);resolve([{jobId:'one',primarySource:'C:/Course/one.pdf',outputPath:'C:/Course/one_Guide.md',courseProfile:'auto'}]);await first;assert.deepEqual(get(pendingFiles),['C:/Course/two.pdf','C:/Course/three.pdf']);assert.equal(get(showConfirmPanel),false);assert.equal(get(selectedJobId),'one');assert.equal(get(confirmationBusy),false);assert.equal(get(jobs)[0].outputPath,'C:/Course/one_Guide.md');assert.equal(calls[0].args.options.requireVisuals,true);});
test('backend rejection keeps sources and releases the submission guard',async()=>{pendingFiles.set(['one.pdf']);handler=async command=>{if(command==='approve_files')throw new Error('source not found');return [];};await submitConfirmation();assert.deepEqual(get(pendingFiles),['one.pdf']);assert.equal(get(confirmationBusy),false);assert.match(get(confirmationError),/source not found/);assert.equal(get(jobs).length,0);});
test('configuration errors are shown without assuming safe defaults',async()=>{confirmationConfig.set(null);handler=async()=>{throw new Error('settings unavailable');};await loadConfirmationConfig();assert.equal(get(confirmationConfig),null);assert.match(get(confirmationError),/settings unavailable/);});
test('resume sends exact saved-context model selections and preserves output path',async()=>{pendingSource.set('resume');pendingFiles.set(['C:/Course/one_Guide.prep.md']);prepEffort.set('low');writerModel.set('claude-opus-5');writerEffort.set('medium');codexFallbackModel.set('gpt-5.6-terra');codexFallbackEffort.set('low');handler=async command=>command==='resume_guide'?'resume-id':[];await submitConfirmation();const call=calls.find(c=>c.command==='resume_guide');assert.equal(call.args.prepPath,'C:/Course/one_Guide.prep.md');assert.deepEqual(call.args.options,{prep:{provider:'codex-chatgpt',model:'gpt-5.6-luna',effort:'low'},collectionFallbacks:[{provider:'codex-chatgpt',model:'gpt-5.6-terra',effort:'medium'},{provider:'codex-chatgpt',model:'gpt-5.6-sol',effort:'medium'}],writer:{provider:'claude-code',model:'claude-opus-5',effort:'medium'},fallbacks:[{provider:'codex-chatgpt',model:'gpt-5.6-terra',effort:'low'}]});assert.equal(get(jobs)[0].outputPath,'C:/Course/one_Guide.md');});
test('optional Claude fallback is serialized before final Codex with exact efforts',async()=>{pendingFiles.set(['C:/Course/one.pdf']);claudeFallbackEnabled.set(true);claudeFallbackModel.set('claude-opus-5');claudeFallbackEffort.set('low');codexFallbackEffort.set('medium');handler=async command=>command==='approve_files'?[{jobId:'one',primarySource:'C:/Course/one.pdf',outputPath:'C:/Course/one_Guide.md',courseProfile:'auto'}]:[];await submitConfirmation();const options=calls.find(c=>c.command==='approve_files').args.options;assert.deepEqual(options.fallbacks,[{provider:'claude-code',model:'claude-opus-5',effort:'low'},{provider:'codex-chatgpt',model:'gpt-5.6-sol',effort:'medium'}]);});
test('config refresh does not overwrite a user selection while the panel is open',()=>{writerModel.set('claude-opus-5');writerEffort.set('low');initializeGenerationSelections(config);assert.equal(get(writerModel),'claude-opus-5');assert.equal(get(writerEffort),'low');});
test('enabling the default optional fallback selects a real alternate Claude model',()=>{initializeGenerationSelections(config,{force:true});assert.equal(get(claudeFallbackEnabled),false);assert.equal(get(claudeFallbackModel),'claude-opus-5');assert.equal(get(claudeFallbackEffort),'high');});
test('saved generation defaults survive reload with exact model and effort values',()=>{prepEffort.set('low');writerModel.set('claude-opus-5');writerEffort.set('medium');codexFallbackModel.set('gpt-5.6-terra');codexFallbackEffort.set('low');saveGenerationPreferences(config);assert(savedPreferences.has(GENERATION_PREFERENCES_KEY));prepEffort.set('max');writerModel.set('claude-opus-4-8');initializeGenerationSelections(config,{force:true});assert.equal(get(prepEffort),'low');assert.equal(get(writerModel),'claude-opus-5');assert.equal(get(writerEffort),'medium');assert.equal(get(codexFallbackModel),'gpt-5.6-terra');assert.equal(get(codexFallbackEffort),'low');});
test('malformed stale duplicate collection and same-model writer fallback preferences are rejected atomically',()=>{const valid={prepModel:'gpt-5.6-luna',prepEffort:'medium',collectionFallback1Model:'gpt-5.6-terra',collectionFallback1Effort:'medium',collectionFallback2Model:'gpt-5.6-sol',collectionFallback2Effort:'medium',writerModel:'claude-opus-4-8',writerEffort:'high',claudeFallbackEnabled:true,claudeFallbackModel:'claude-opus-5',claudeFallbackEffort:'low',codexFallbackModel:'gpt-5.6-sol',codexFallbackEffort:'medium',selectedCourse:'auto'};assert(normalizeGenerationPreferences(config,valid));assert.equal(normalizeGenerationPreferences(config,{...valid,prepEffort:'ultra'}),null);assert.equal(normalizeGenerationPreferences(config,{...valid,collectionFallback1Model:'gpt-5.6-luna'}),null);assert.equal(normalizeGenerationPreferences(config,{...valid,claudeFallbackModel:'claude-opus-4-8'}),null);savedPreferences.set(GENERATION_PREFERENCES_KEY,'{broken');initializeGenerationSelections(config,{force:true});assert.equal(get(prepEffort),'medium');assert.equal(get(writerModel),'claude-opus-4-8');});
test('reset removes persisted settings and restores the canonical economical defaults',()=>{writerModel.set('claude-opus-5');writerEffort.set('max');saveGenerationPreferences(config);resetGenerationPreferences(config);assert.equal(savedPreferences.has(GENERATION_PREFERENCES_KEY),false);assert.equal(get(prepModel),'gpt-5.6-luna');assert.equal(get(prepEffort),'medium');assert.equal(get(collectionFallback1Model),'gpt-5.6-terra');assert.equal(get(collectionFallback1Effort),'medium');assert.equal(get(collectionFallback2Model),'gpt-5.6-sol');assert.equal(get(collectionFallback2Effort),'medium');assert.equal(get(writerModel),'claude-opus-4-8');assert.equal(get(writerEffort),'high');assert.equal(get(claudeFallbackEnabled),false);});

test('a quality preset moves only effort levels and reports itself back',()=>{
 // Models are an advanced decision: a preset must never silently change who writes.
 writerModel.set('claude-opus-5');prepModel.set('gpt-5.6-terra');
 assert.equal(applyQualityPreset(config,'thorough'),'thorough');
 assert.equal(get(writerModel),'claude-opus-5');
 assert.equal(get(prepModel),'gpt-5.6-terra');
 assert.equal(get(prepEffort),'max');
 assert.equal(get(writerEffort),'xhigh');
 assert.equal(activeQualityPreset(),'thorough');
 assert.equal(applyQualityPreset(config,'quick'),'quick');
 assert.equal(get(prepEffort),'low');
 assert.equal(get(writerEffort),'medium');
 assert.equal(activeQualityPreset(),'quick');
 // Balanced is the app's own default effort set, so a fresh install already matches it.
 initializeGenerationSelections(config,{force:true});
 assert.equal(activeQualityPreset(),'balanced');
 // A hand-picked effort is honestly reported as custom rather than snapped to a preset.
 writerEffort.set('max');
 assert.equal(activeQualityPreset(),'custom');
 // An unknown preset, or a provider that does not offer the efforts, changes nothing.
 writerEffort.set('high');
 assert.equal(applyQualityPreset(config,'nonsense'),null);
 assert.equal(get(writerEffort),'high');
 // A provider offering only one effort no preset asks for: every preset drops out.
 const thin={...config,providers:[{id:'codex-chatgpt',models:config.providers[0].models,efforts:['medium']},{id:'claude-code',models:config.providers[1].models,efforts:['low']}]};
 assert.equal(applyQualityPreset(thin,'thorough'),null);
 assert.equal(get(writerEffort),'high');
 assert.deepEqual(availableQualityPresets(thin).map(preset=>preset.id),[]);
 assert.deepEqual(availableQualityPresets(config).map(preset=>preset.id),QUALITY_PRESETS.map(preset=>preset.id));
 assert.deepEqual(availableQualityPresets(null),[]);
});

test('a preset keeps the selections valid enough to save and send',()=>{
 for(const preset of QUALITY_PRESETS){
  applyQualityPreset(config,preset.id);
  assert(normalizeGenerationPreferences(config,generationSelectionSnapshot()),preset.id+' must stay saveable');
 }
});

