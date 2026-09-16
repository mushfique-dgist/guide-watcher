import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { pendingFiles,pendingSource,showConfirmPanel,addJob,selectedJobId,revealDeferredWatcherFiles } from '../stores/jobs.js';
import { currentView,busyAction,actionNotice } from '../stores/navigation.js';
import { refreshHistory } from '../stores/history.js';
import { filename,folder } from './ui.js';
import { buildGenerateOptions,buildResumePrepOptions,PREP_PROVIDER,WRITER_PROVIDER } from './jobRequests.js';
export const confirmationConfig=writable(null),confirmationError=writable(''),confirmationBusy=writable(false),sourceChoices=writable({}),selectedCourse=writable('auto');
export const prepModel=writable(''),prepEffort=writable(''),writerModel=writable(''),writerEffort=writable('');
export const collectionFallback1Model=writable(''),collectionFallback1Effort=writable('');
export const collectionFallback2Model=writable(''),collectionFallback2Effort=writable('');
export const claudeFallbackEnabled=writable(false),claudeFallbackModel=writable(''),claudeFallbackEffort=writable('');
export const codexFallbackModel=writable(''),codexFallbackEffort=writable('');
let loading=null;
let generationSelectionsInitialized=false;
export const GENERATION_PREFERENCES_KEY='guide-watcher-generation-preferences-v2';
export const sourceKey=path=>path.replaceAll(String.fromCharCode(92),'/').toLowerCase();
export function isChosen(path,choices=get(sourceChoices)){return choices[sourceKey(path)]!==false;}
export function setChosen(paths,value){sourceChoices.update(old=>{const next={...old};for(const path of paths)next[sourceKey(path)]=value;return next;});}
function browserStorage(){const storage=globalThis.localStorage;return storage&&typeof storage.getItem==='function'&&typeof storage.setItem==='function'?storage:null;}
function configuredProvider(config,id){return config.providers.find(provider=>provider.id===id);}
function validChoice(options,value){return typeof value==='string'&&options.includes(value);}
function defaultSelections(config){
 const collection=config.collection_fallback_chain;
 if(!Array.isArray(collection)||collection.length!==2)throw new Error('Generation configuration must provide two collection fallbacks.');
 const claude=config.fallback_chain.find(item=>item.provider==='claude-code');
 const codex=[...config.fallback_chain].reverse().find(item=>item.provider==='codex-chatgpt');
 const claudeProvider=configuredProvider(config,'claude-code');
 const alternateClaudeModel=claudeProvider?.models?.find(model=>model!==config.default_writer.model);
 if(!codex)throw new Error('Generation configuration has no final Codex fallback.');
 return {prepModel:config.default_prep.model,prepEffort:config.default_prep.effort,collectionFallback1Model:collection[0].model,collectionFallback1Effort:collection[0].effort,collectionFallback2Model:collection[1].model,collectionFallback2Effort:collection[1].effort,writerModel:config.default_writer.model,writerEffort:config.default_writer.effort,claudeFallbackEnabled:Boolean(claude),claudeFallbackModel:claude?.model||alternateClaudeModel||config.default_writer.model,claudeFallbackEffort:claude?.effort||config.default_writer.effort,codexFallbackModel:codex.model,codexFallbackEffort:codex.effort,selectedCourse:config.default_course_profile||'auto'};
}
export function normalizeGenerationPreferences(config,value){
 if(!value||typeof value!=='object'||Array.isArray(value))return null;
 const codex=configuredProvider(config,'codex-chatgpt'),claude=configuredProvider(config,'claude-code');
 if(!codex||!claude||typeof value.claudeFallbackEnabled!=='boolean')return null;
 if(!validChoice(codex.models,value.prepModel)||!validChoice(codex.efforts,value.prepEffort)||!validChoice(codex.models,value.collectionFallback1Model)||!validChoice(codex.efforts,value.collectionFallback1Effort)||!validChoice(codex.models,value.collectionFallback2Model)||!validChoice(codex.efforts,value.collectionFallback2Effort)||!validChoice(claude.models,value.writerModel)||!validChoice(claude.efforts,value.writerEffort)||!validChoice(claude.models,value.claudeFallbackModel)||!validChoice(claude.efforts,value.claudeFallbackEffort)||!validChoice(codex.models,value.codexFallbackModel)||!validChoice(codex.efforts,value.codexFallbackEffort))return null;
 if(new Set([value.prepModel,value.collectionFallback1Model,value.collectionFallback2Model]).size!==3)return null;
 if(value.claudeFallbackEnabled&&value.claudeFallbackModel===value.writerModel)return null;
 const courses=new Set(['auto',...(config.course_profiles||[]).map(profile=>profile.id)]);
 if(typeof value.selectedCourse!=='string'||!courses.has(value.selectedCourse))return null;
 return {prepModel:value.prepModel,prepEffort:value.prepEffort,collectionFallback1Model:value.collectionFallback1Model,collectionFallback1Effort:value.collectionFallback1Effort,collectionFallback2Model:value.collectionFallback2Model,collectionFallback2Effort:value.collectionFallback2Effort,writerModel:value.writerModel,writerEffort:value.writerEffort,claudeFallbackEnabled:value.claudeFallbackEnabled,claudeFallbackModel:value.claudeFallbackModel,claudeFallbackEffort:value.claudeFallbackEffort,codexFallbackModel:value.codexFallbackModel,codexFallbackEffort:value.codexFallbackEffort,selectedCourse:value.selectedCourse};
}
function readGenerationPreferences(config){
 try {const raw=browserStorage()?.getItem(GENERATION_PREFERENCES_KEY);return raw?normalizeGenerationPreferences(config,JSON.parse(raw)):null;} catch {return null;}
}
function applySelections(value){
 prepModel.set(value.prepModel);prepEffort.set(value.prepEffort);writerModel.set(value.writerModel);writerEffort.set(value.writerEffort);
 collectionFallback1Model.set(value.collectionFallback1Model);collectionFallback1Effort.set(value.collectionFallback1Effort);collectionFallback2Model.set(value.collectionFallback2Model);collectionFallback2Effort.set(value.collectionFallback2Effort);
 claudeFallbackEnabled.set(value.claudeFallbackEnabled);claudeFallbackModel.set(value.claudeFallbackModel);claudeFallbackEffort.set(value.claudeFallbackEffort);
 codexFallbackModel.set(value.codexFallbackModel);codexFallbackEffort.set(value.codexFallbackEffort);selectedCourse.set(value.selectedCourse);
}
// How thorough to be, as one plain choice. A preset moves only the effort levels, never the
// models: which model writes is an advanced decision that stays in Settings, and every effort
// below is one the providers accept. Balanced matches the app's own defaults.
export const QUALITY_PRESETS=[
 {id:'quick',label:'Quick',note:'Fewer passes over the material. Good for a light lecture or a first look.',
  efforts:{prep:'low',collection1:'low',collection2:'medium',writer:'medium',claudeFallback:'medium',codexFallback:'low'}},
 {id:'balanced',label:'Balanced',note:'The recommended setting for a normal lecture.',
  efforts:{prep:'medium',collection1:'medium',collection2:'medium',writer:'high',claudeFallback:'high',codexFallback:'medium'}},
 {id:'thorough',label:'Thorough',note:'Reads harder and writes longer. Best for dense or exam-critical lectures.',
  efforts:{prep:'max',collection1:'high',collection2:'high',writer:'xhigh',claudeFallback:'xhigh',codexFallback:'high'}},
];
const EFFORT_STORES={prep:prepEffort,collection1:collectionFallback1Effort,collection2:collectionFallback2Effort,writer:writerEffort,claudeFallback:claudeFallbackEffort,codexFallback:codexFallbackEffort};
function supportedEfforts(config){
 const codex=configuredProvider(config,'codex-chatgpt'),claude=configuredProvider(config,'claude-code');
 return {codex:codex?.efforts||[],claude:claude?.efforts||[]};
}
function presetIsSupported(config,preset){
 const {codex,claude}=supportedEfforts(config);
 const {prep,collection1,collection2,writer,claudeFallback,codexFallback}=preset.efforts;
 return [prep,collection1,collection2,codexFallback].every(effort=>codex.includes(effort))
  &&[writer,claudeFallback].every(effort=>claude.includes(effort));
}
export function availableQualityPresets(config){
 return config?QUALITY_PRESETS.filter(preset=>presetIsSupported(config,preset)):[];
}
// Apply a preset to the current selections. Models are left exactly as they are.
export function applyQualityPreset(config,id){
 const preset=QUALITY_PRESETS.find(item=>item.id===id);
 if(!preset||!config||!presetIsSupported(config,preset))return null;
 for(const [key,store] of Object.entries(EFFORT_STORES))store.set(preset.efforts[key]);
 return preset.id;
}
// Which preset the current selections match, or 'custom' when they match none. Fallback
// efforts that are switched off do not count against a match.
export function activeQualityPreset(selections=generationSelectionSnapshot()){
 const match=QUALITY_PRESETS.find(preset=>
  selections.prepEffort===preset.efforts.prep
  &&selections.collectionFallback1Effort===preset.efforts.collection1
  &&selections.collectionFallback2Effort===preset.efforts.collection2
  &&selections.writerEffort===preset.efforts.writer
  &&selections.codexFallbackEffort===preset.efforts.codexFallback
  &&(!selections.claudeFallbackEnabled||selections.claudeFallbackEffort===preset.efforts.claudeFallback));
 return match?match.id:'custom';
}
export function generationSelectionSnapshot(){return {prepModel:get(prepModel),prepEffort:get(prepEffort),collectionFallback1Model:get(collectionFallback1Model),collectionFallback1Effort:get(collectionFallback1Effort),collectionFallback2Model:get(collectionFallback2Model),collectionFallback2Effort:get(collectionFallback2Effort),writerModel:get(writerModel),writerEffort:get(writerEffort),claudeFallbackEnabled:get(claudeFallbackEnabled),claudeFallbackModel:get(claudeFallbackModel),claudeFallbackEffort:get(claudeFallbackEffort),codexFallbackModel:get(codexFallbackModel),codexFallbackEffort:get(codexFallbackEffort),selectedCourse:get(selectedCourse)};}
export function saveGenerationPreferences(config,value=generationSelectionSnapshot()){
 const normalized=normalizeGenerationPreferences(config,value);
 if(!normalized)throw new Error('Choose supported models and efforts. Collection models must be distinct, and an enabled Claude fallback must differ from the primary writer.');
 const storage=browserStorage();if(!storage)throw new Error('Settings storage is unavailable.');
 storage.setItem(GENERATION_PREFERENCES_KEY,JSON.stringify(normalized));applySelections(normalized);return normalized;
}
export function resetGenerationPreferences(config){
 browserStorage()?.removeItem(GENERATION_PREFERENCES_KEY);const defaults=defaultSelections(config);applySelections(defaults);generationSelectionsInitialized=true;return defaults;
}
export function initializeGenerationSelections(config,{force=false}={}){
 if(generationSelectionsInitialized&&!force)return;
 applySelections(readGenerationPreferences(config)||defaultSelections(config));
 generationSelectionsInitialized=true;
}
export function loadConfirmationConfig(){if(loading)return loading;confirmationConfig.set(null);confirmationError.set('');loading=invoke('get_config').then(config=>{if(!config?.default_prep?.model||!config?.default_writer?.model||!Array.isArray(config.providers)||!Array.isArray(config.collection_fallback_chain)||!Array.isArray(config.fallback_chain)||!Array.isArray(config.course_profiles))throw new Error('Generation configuration is incomplete. Restart the app or repair its configuration.');initializeGenerationSelections(config);confirmationConfig.set(config);}).catch(error=>confirmationError.set(String(error))).finally(()=>loading=null);return loading;}
// Resume options from the current generation preferences (Settings or the last confirmation
// panel). Loads the configuration first so the selections are initialized.
export async function currentResumeOptions(){
 if(!get(confirmationConfig))await loadConfirmationConfig();
 if(!get(confirmationConfig))return null;
 const fallbacks=[];
 if(get(claudeFallbackEnabled))fallbacks.push({model:get(claudeFallbackModel),effort:get(claudeFallbackEffort)});
 fallbacks.push({model:get(codexFallbackModel),effort:get(codexFallbackEffort)});
 const collectionFallbacks=[{model:get(collectionFallback1Model),effort:get(collectionFallback1Effort)},{model:get(collectionFallback2Model),effort:get(collectionFallback2Effort)}];
 return buildResumePrepOptions({prepModel:get(prepModel),prepEffort:get(prepEffort),collectionFallbacks,writerModel:get(writerModel),writerEffort:get(writerEffort),fallbacks});
}
export function discardSelection(){if(get(confirmationBusy))return;pendingFiles.set([]);sourceChoices.set({});showConfirmPanel.set(false);confirmationError.set('');revealDeferredWatcherFiles();}
export async function submitConfirmation(){
 if(get(confirmationBusy)||get(busyAction))return;
 const config=get(confirmationConfig),mode=get(pendingSource),all=get(pendingFiles),selected=mode==='resume'?all.slice(0,1):all.filter(path=>isChosen(path));
 if(!config){confirmationError.set('Wait for generation settings to load before continuing.');return;}if(!selected.length){confirmationError.set('Select at least one source to continue.');return;}
 confirmationBusy.set(true);busyAction.set('generate');confirmationError.set('');
 const fallbacks=[];
 if(get(claudeFallbackEnabled))fallbacks.push({model:get(claudeFallbackModel),effort:get(claudeFallbackEffort)});
 fallbacks.push({model:get(codexFallbackModel),effort:get(codexFallbackEffort)});
 const collectionFallbacks=[{model:get(collectionFallback1Model),effort:get(collectionFallback1Effort)},{model:get(collectionFallback2Model),effort:get(collectionFallback2Effort)}];
 const input={prepModel:get(prepModel),prepEffort:get(prepEffort),collectionFallbacks,writerModel:get(writerModel),writerEffort:get(writerEffort),fallbacks,courseProfile:get(selectedCourse),requireSlideCoverage:true,requireVisuals:true};
 try {
  let planned;
  if(mode==='resume'){const jobId=await invoke('resume_guide',{prepPath:selected[0],options:buildResumePrepOptions(input)});planned=[{jobId,primarySource:selected[0],outputPath:selected[0].replace(/\.prep\.md$/i,'.md'),courseProfile:input.courseProfile}];}
  else planned=await invoke('approve_files',{filePaths:selected,options:buildGenerateOptions(input)});
  if(!Array.isArray(planned)||!planned.length||planned.some(job=>!job.jobId))throw new Error('The app did not return a valid job. Check History before trying again.');
  for(const job of planned)addJob({id:job.jobId,filename:filename(job.primarySource),filepath:job.primarySource,folder:job.courseLabel||folder(job.primarySource),outputName:filename(job.outputPath),outputPath:job.outputPath,prepPath:mode==='resume'?selected[0]:job.outputPath?.replace(/\.md$/i,'.prep.md'),status:'starting',activity:mode==='resume'?'Preparing saved context for writing.':'Sources accepted. Preparing context collection.',provider:WRITER_PROVIDER,model:input.writerModel,effort:input.writerEffort,prepProvider:PREP_PROVIDER,prepModel:input.prepModel,prepEffort:input.prepEffort,collectionFallbackChain:input.collectionFallbacks,fallbackChain:input.fallbacks,courseProfile:job.courseProfile,requireSlideCoverage:true,requireVisuals:true,startedAt:Date.now(),finishedAt:null});
  const removed=new Set(selected.map(sourceKey));pendingFiles.update(paths=>paths.filter(path=>!removed.has(sourceKey(path))));sourceChoices.update(choices=>Object.fromEntries(Object.entries(choices).filter(([key])=>!removed.has(key))));
  showConfirmPanel.set(false);selectedJobId.set(planned[0].jobId);currentView.set('run');revealDeferredWatcherFiles();
  if(get(pendingFiles).length){showConfirmPanel.set(false);actionNotice.set('Additional sources remain under Review sources.');}
  await refreshHistory();
 }catch(error){confirmationError.set(String(error));await refreshHistory();}finally{confirmationBusy.set(false);if(get(busyAction)==='generate')busyAction.set('');}
}
