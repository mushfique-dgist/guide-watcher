<script>
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { currentView } from '../stores/navigation.js';
  import { jobs } from '../stores/jobs.js';
  import { providerAuthSessions, rememberProviderAuthStart } from '../stores/providerAuth.js';
  import {
    initializeGenerationSelections, generationSelectionSnapshot,
    saveGenerationPreferences, resetGenerationPreferences,
    availableQualityPresets, applyQualityPreset, activeQualityPreset,
  } from './confirmation.js';
  import { powerMode, setPowerMode } from '../stores/preferences.js';
  import Setup from './Setup.svelte';

  let { scale=1, setScale, theme='dark', setTheme, resetLayout } = $props();
  let config=$state(null); let configError=$state(''); let settingsNotice=$state('');
  let showSetup=$state(false);
  let draft=$state(null);
  let authStatuses=$state([]); let authError=$state(''); let authBusy=$state(false); let copiedSession=$state('');
  const codexOptions=$derived(config?.providers?.find(provider=>provider.id==='codex-chatgpt'));
  const claudeOptions=$derived(config?.providers?.find(provider=>provider.id==='claude-code'));

  const presets=$derived(availableQualityPresets(config));
  let presetChoice=$state('balanced');
  function choosePreset(id){if(!config)return;applyQualityPreset(config,id);draft=generationSelectionSnapshot();presetChoice=activeQualityPreset(draft);settingsNotice='';saveDefaults();}
  function statusFor(provider){return authStatuses.find(item=>item.provider===provider);}
  function activeSession(provider){const session=$providerAuthSessions[provider];return session&&['starting','waiting'].includes(session.state)?session:null;}
  async function loadConfig(){configError='';try{config=await invoke('get_config');initializeGenerationSelections(config);draft=generationSelectionSnapshot();presetChoice=activeQualityPreset(draft);}catch(error){configError=String(error);}}
  async function refreshAuth(){authBusy=true;authError='';try{authStatuses=await invoke('get_provider_auth_statuses');}catch(error){authError=String(error);}finally{authBusy=false;}}
  async function beginLogin(provider){authError='';try{const session=await invoke('start_provider_login',{provider});rememberProviderAuthStart(session);}catch(error){authError=String(error);}}
  async function cancelLogin(provider){const session=$providerAuthSessions[provider];if(!session)return;try{await invoke('cancel_provider_login',{provider,sessionId:session.sessionId});}catch(error){authError=String(error);}}
  async function openVerification(uri){try{await openUrl(uri);}catch(error){authError=`Could not open the sign-in page: ${error}`;}}
  async function copyCodeAndOpen(session){
    authError='';
    try{await writeText(session.userCode,{label:'Guide Watcher one-time device code'});copiedSession=session.sessionId;}
    catch(error){authError=`Could not copy the device code. Select it in the app and copy it manually: ${error}`;return;}
    await openVerification(session.verificationUri);
  }
  function saveDefaults(){settingsNotice='';try{draft=saveGenerationPreferences(config,draft);settingsNotice='Generation defaults saved. New guides and resumes will start with these choices.';}catch(error){settingsNotice=String(error);}}
  function restoreDefaults(){draft=resetGenerationPreferences(config);settingsNotice='Restored Luna medium, Terra medium, then Sol medium for collection; Opus 4.8 high for writing; and Sol medium for final fallback.';}

  onMount(()=>{
    let unsubscribe=()=>{};let disposed=false;
    loadConfig();refreshAuth();
    listen('provider-auth',event=>{
      if(disposed||!event?.payload?.provider)return;
      const update=event.payload;
      if(['succeeded','failed','cancelled'].includes(update.state))refreshAuth();
    }).then(stop=>{if(disposed)stop();else unsubscribe=stop;}).catch(error=>authError=String(error));
    return()=>{disposed=true;unsubscribe();};
  });
</script>

<section class="page" aria-labelledby="settings-title"><div class="page-inner settings">
<div class="page-heading"><div><h1 id="settings-title">Settings & help</h1><p>Manage provider accounts, choose generation defaults, and adjust the workspace.</p></div></div>

<section><div class="setting"><div><label for="power-mode">Power mode</label>
<p>Off, the app keeps the daily screens plain: one choice of how thorough to be, and progress in four steps.
On, every model, effort and fallback appears on the screens where it applies, with the full activity log.
Both run exactly the same pipeline and produce the same guides.</p></div>
<select id="power-mode" value={$powerMode?'on':'off'} onchange={e=>setPowerMode(e.currentTarget.value==='on')}><option value="off">Off · simple</option><option value="on">On · everything visible</option></select></div></section>

<section><div class="section-heading"><div><h2>Model accounts</h2><p>Guide Watcher starts each provider’s native sign-in. Codex and Claude keep the credentials; the app never asks for or stores passwords, tokens, or API keys.</p></div><button class="button ghost" disabled={authBusy} onclick={refreshAuth}>{authBusy?'Checking…':'Refresh status'}</button></div>
{#if authError}<p class="error-box" role="alert">{authError}</p>{/if}
<div class="account-list" aria-busy={authBusy}>
{#each ['codex-chatgpt','claude-code'] as provider}
  {@const status=statusFor(provider)}{@const session=activeSession(provider)}{@const latest=$providerAuthSessions[provider]}
  <article class="account-card">
    <div class="account-main"><div><h3>{status?.label||(provider==='codex-chatgpt'?'OpenAI Codex':'Claude Code')}</h3><p>{status?.accountLabel||status?.authMethod||(status?.installed===false?'CLI not installed':'Account status unavailable')}</p></div><span class:connected={status?.authenticated} class="account-state">{status?.authenticated?'Connected':'Not connected'}</span></div>
    <p class="account-detail">{status?.detail||'Checking the native CLI…'}</p>
    {#if latest?.message}<p class:auth-success={latest.state==='succeeded'} class:auth-failure={latest.state==='failed'} class="auth-message" role="status">{latest.message}</p>{/if}
    {#if latest?.userCode}<div class="device-code"><label for={`device-code-${provider}`}>One-time device code</label><input id={`device-code-${provider}`} class="device-code-input" value={latest.userCode} readonly spellcheck="false" onfocus={event=>event.currentTarget.select()} onclick={event=>event.currentTarget.select()}/><span>Expires after about 15 minutes.</span></div>{/if}
    <div class="account-actions">
      {#if latest?.verificationUri && latest?.userCode}<button class="button primary" onclick={()=>copyCodeAndOpen(latest)}>{copiedSession===latest.sessionId?'Code copied — open page again':'Copy code and open sign-in page'}</button>
      {:else if latest?.verificationUri}<button class="button primary" disabled>Waiting for device code…</button>{/if}
      {#if session}<button class="button secondary" onclick={()=>cancelLogin(provider)}>Cancel sign-in</button>
      {:else}<button class="button secondary" disabled={status?.installed===false} onclick={()=>beginLogin(provider)}>{status?.authenticated?'Switch account':'Sign in'}</button>{/if}
    </div>
  </article>
{/each}
</div></section>

<section><h2>{$powerMode?'Generation defaults':'How thorough to be'}</h2><p class="muted">{$powerMode?'These are the starting choices for every new guide. You can still change them for one run on the source review screen.':'The starting point for every new guide. You can still change it for one run before it starts.'}</p>
{#if !$powerMode&&config&&draft}
<div class="preset-row" role="radiogroup" aria-label="How thorough to be">
{#each presets as preset}<button type="button" class="preset" class:chosen={presetChoice===preset.id} role="radio" aria-checked={presetChoice===preset.id} onclick={()=>choosePreset(preset.id)}><strong>{preset.label}</strong><span>{preset.note}</span></button>{/each}
</div>
{#if presetChoice==='custom'}<p class="muted">Your saved settings are a custom mix. Choosing one above replaces it.</p>{/if}
<p class="muted">Which models do the work is a Power mode setting. Turn Power mode on above to choose them.</p>
{/if}
{#if configError}<p class="error-box" role="alert">Could not load configuration: {configError}</p>{:else if !config||!draft}<p role="status">Loading configuration…</p>{:else if codexOptions&&claudeOptions&&$powerMode}
<div class="model-grid">
<fieldset><legend>Context collection</legend><label for="settings-prep-model">Primary Codex model</label><select id="settings-prep-model" bind:value={draft.prepModel}>{#each codexOptions.models as model}<option value={model}>{model}</option>{/each}</select><label for="settings-prep-effort">Primary effort</label><select id="settings-prep-effort" bind:value={draft.prepEffort}>{#each codexOptions.efforts as effort}<option value={effort}>{effort}</option>{/each}</select><p>Used first for each visual batch and evidence packet.</p></fieldset>
<fieldset><legend>Collection recovery</legend><label for="settings-collection-fallback-1-model">First fallback</label><select id="settings-collection-fallback-1-model" bind:value={draft.collectionFallback1Model}>{#each codexOptions.models as model}<option value={model}>{model}</option>{/each}</select><label for="settings-collection-fallback-1-effort">First fallback effort</label><select id="settings-collection-fallback-1-effort" bind:value={draft.collectionFallback1Effort}>{#each codexOptions.efforts as effort}<option value={effort}>{effort}</option>{/each}</select><label for="settings-collection-fallback-2-model">Second fallback</label><select id="settings-collection-fallback-2-model" bind:value={draft.collectionFallback2Model}>{#each codexOptions.models as model}<option value={model}>{model}</option>{/each}</select><label for="settings-collection-fallback-2-effort">Second fallback effort</label><select id="settings-collection-fallback-2-effort" bind:value={draft.collectionFallback2Effort}>{#each codexOptions.efforts as effort}<option value={effort}>{effort}</option>{/each}</select><p>Rejected or unavailable results move forward one model. Accepted units are kept.</p></fieldset>
<fieldset><legend>Guide writing</legend><label for="settings-writer-model">Claude model</label><select id="settings-writer-model" bind:value={draft.writerModel}>{#each claudeOptions.models as model}<option value={model}>{model}</option>{/each}</select><label for="settings-writer-effort">Effort</label><select id="settings-writer-effort" bind:value={draft.writerEffort}>{#each claudeOptions.efforts as effort}<option value={effort}>{effort}</option>{/each}</select><p>The selected effort is passed directly to Claude Code.</p></fieldset>
<fieldset><legend>Optional Claude fallback</legend><label class="toggle-row"><input type="checkbox" bind:checked={draft.claudeFallbackEnabled}/>Try a second Claude model first</label><label for="settings-claude-fallback-model">Fallback model</label><select id="settings-claude-fallback-model" bind:value={draft.claudeFallbackModel} disabled={!draft.claudeFallbackEnabled}>{#each claudeOptions.models as model}<option value={model}>{model}</option>{/each}</select><label for="settings-claude-fallback-effort">Fallback effort</label><select id="settings-claude-fallback-effort" bind:value={draft.claudeFallbackEffort} disabled={!draft.claudeFallbackEnabled}>{#each claudeOptions.efforts as effort}<option value={effort}>{effort}</option>{/each}</select><p>Use a different model from the primary Claude writer.</p></fieldset>
<fieldset><legend>Final fallback</legend><label for="settings-codex-fallback-model">Codex model</label><select id="settings-codex-fallback-model" bind:value={draft.codexFallbackModel}>{#each codexOptions.models as model}<option value={model}>{model}</option>{/each}</select><label for="settings-codex-fallback-effort">Reasoning effort</label><select id="settings-codex-fallback-effort" bind:value={draft.codexFallbackEffort}>{#each codexOptions.efforts as effort}<option value={effort}>{effort}</option>{/each}</select><p>If Claude exhausts its allowance, this Codex model continues writing and repair.</p></fieldset>
</div>
<div class="settings-actions"><button class="button primary" onclick={saveDefaults}>Save generation defaults</button><button class="button ghost" onclick={restoreDefaults}>Restore recommended defaults</button></div>
{#if settingsNotice}<p class:save-error={!settingsNotice.startsWith('Generation defaults saved')&&!settingsNotice.startsWith('Restored')} class="settings-notice" role="status">{settingsNotice}</p>{/if}
<p class="muted">Low, medium, high, xhigh, and max are distinct CLI effort settings. Guide Watcher sends the selected value unchanged; source coverage, visual analysis, and native verification stay mandatory.</p>
{:else if !$powerMode}{:else}<p role="alert">The configured model providers are incomplete.</p>{/if}</section>

<section><h2>This computer</h2><div class="setting"><div><strong>Folders and subjects</strong><p>Where your course material lives, which subjects you keep, and which files start a guide. Every answer is checked against this computer before it is saved.</p></div><button class="button secondary" onclick={()=>{showSetup=true;}}>Open setup</button></div></section>
<section><h2>Appearance</h2><div class="setting"><div><label for="theme">Theme</label><p>Choose a comfortable reading surface.</p></div><select id="theme" value={theme} onchange={e=>setTheme(e.currentTarget.value)}><option value="dark">Dark</option><option value="light">Light</option><option value="system">Use system setting</option></select></div><div class="setting"><div><label for="scale">Interface size</label><p>Ctrl + / Ctrl − to adjust. Ctrl 0 returns to 100%.</p></div><select id="scale" value={scale} onchange={e=>setScale(Number(e.currentTarget.value))}>{#each [...new Set([.85,1,1.1,1.25,1.5,1.75,2,scale])].sort((a,b)=>a-b) as value}<option value={value}>{Math.round(value*100)}%</option>{/each}</select></div><div class="setting"><div><strong>Workspace layout</strong><p>Restore the default sidebar width and interface size.</p></div><button class="button secondary" onclick={resetLayout}>Reset layout</button></div></section>
<section><h2>History</h2><div class="setting"><div><strong>Full history</strong><p>Find attempts removed from Recents. Restore a record or permanently delete its history entry. Source files and saved guides are kept.</p><p>{$jobs.filter(j=>j.archivedAt).length} archived · {$jobs.length} total attempts</p></div><button class="button secondary" onclick={()=>{$currentView='full-history';}}>Open full history</button></div></section>
<section><h2>Understanding your results</h2><details><summary>What does “Succeeded” mean?</summary><p>The generation process finished and the app verified the published output. A provider finishing its response does not by itself mean the guide is complete.</p></details><details><summary>Should I retry or resume?</summary><p>Resume uses a saved, verified context packet to continue writing. Retry starts a new attempt from the source material. History offers an action only when the required files are available and the app supports it.</p></details><details><summary>What happens when I close the app?</summary><p>The window’s Close button stops active jobs and sign-in sessions, then hides the app in the system tray. Minimize keeps work running. Use Quit in the tray menu to exit. Recorded attempts stay in History; unfinished work is never shown as successful.</p></details><details><summary>Where are my older runs?</summary><p>The app imports older valid prep packets, verified guide receipts, and retained unfinished workspaces. Imported entries explain what the files establish; lost logs and unknown failure reasons cannot be reconstructed. New desktop attempts have a saved timeline.</p></details><details><summary>Keyboard shortcuts</summary><p>Ctrl N: choose source files. Ctrl H: history. Ctrl + / Ctrl −: interface size. Ctrl 0: reset size. Tab and Shift Tab: move between controls. Escape: close the navigation drawer. Use arrow keys on the sidebar divider to resize it.</p></details></section>
</div></section>
{#if showSetup}<Setup edit onready={()=>{showSetup=false;}} oncancel={()=>{showSetup=false;}}/>{/if}

<style>
.preset-row{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:.65rem;}
.preset{display:grid;gap:.3rem;text-align:left;padding:.85rem;border:1px solid var(--border-panel);border-radius:var(--radius);background:var(--bg-panel);}
.preset:hover{border-color:var(--border-subtle);}
.preset.chosen{border-color:var(--brand);background:var(--bg-active);}
.preset strong{font-weight:600;}
.preset span{font-size:.85rem;color:var(--text-secondary);line-height:1.45;}
@container(max-width:600px){.preset-row{grid-template-columns:1fr;}}
.settings{max-width:860px;}.settings>section{border-top:1px solid var(--border-panel);padding:1.5rem 0;display:grid;gap:1rem;}.section-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:1rem;}.section-heading>.button{white-space:nowrap;flex:none;}.section-heading p,.model-grid p{font-size:.88rem;color:var(--text-secondary);margin-top:.3rem;line-height:1.5;}.account-list{display:grid;gap:.75rem;}.account-card{border:1px solid var(--border-panel);border-radius:var(--radius);padding:1rem;display:grid;gap:.75rem;background:var(--bg-panel);}.account-main,.account-actions,.settings-actions{display:flex;align-items:center;justify-content:space-between;gap:.75rem;flex-wrap:wrap;}.account-main p,.account-detail{font-size:.9rem;color:var(--text-secondary);overflow-wrap:anywhere;}.account-state{font-size:.8rem;border:1px solid var(--border-subtle);border-radius:5px;padding:.2rem .55rem;color:var(--accent-yellow);}.account-state.connected{color:var(--accent-green);}.auth-message{font-size:.9rem;color:var(--accent-blue);}.auth-message.auth-success{color:var(--accent-green);}.auth-message.auth-failure,.save-error{color:var(--accent-red);}.device-code{display:grid;grid-template-columns:max-content minmax(10rem,14rem) 1fr;align-items:center;gap:.5rem .75rem;color:var(--text-secondary);font-size:.85rem;}.device-code-input{font-family:var(--font-mono);font-size:1.05rem;font-weight:700;letter-spacing:.12em;color:var(--text-primary);background:var(--bg-surface);border:1px solid var(--border-subtle);border-radius:5px;padding:.55rem .65rem;width:100%;}.model-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1rem;}.model-grid fieldset{display:grid;gap:.55rem;margin:0;padding:1rem;border:1px solid var(--border-panel);border-radius:var(--radius);min-width:0;}.model-grid legend{padding:0 .35rem;font-weight:600;}.toggle-row{display:flex;align-items:flex-start;gap:.55rem;line-height:1.35;}.toggle-row input{margin-top:.15rem;}.settings-notice{font-size:.9rem;color:var(--accent-green);}.setting{display:flex;gap:1rem;align-items:center;justify-content:space-between;padding:.5rem 0;}.setting>div{min-width:0;}.setting p{font-size:.9rem;color:var(--text-secondary);margin-top:.25rem;}label{font-weight:600;}select{min-width:9rem;}details{border-bottom:1px solid var(--border-panel);padding:.65rem 0;}summary{font-weight:500;}details p{color:var(--text-secondary);margin-top:.75rem;max-width:65ch;}@container(max-width:600px){.setting,.section-heading{align-items:flex-start;flex-direction:column;}.model-grid{grid-template-columns:1fr;}.device-code{grid-template-columns:1fr;}.account-actions .button,.settings-actions .button{flex:1;}}
</style>
