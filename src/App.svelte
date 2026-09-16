<script>
  import { onMount, tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { parseLineEvents } from './lib/parser.js';
  import { updateJobStatus,updateJobActivity,appendStep,receiveWatcherFiles,selectedJobId,showConfirmPanel,pendingFiles } from './stores/jobs.js';
  import { currentView,actionError,actionNotice } from './stores/navigation.js';
  import { chooseSources } from './lib/sourceActions.js';
  import { normalizeScale,clampSidebar,readableError } from './lib/ui.js';
  import { refreshHistory } from './stores/history.js';
  import { applyProviderAuthEvent } from './stores/providerAuth.js';
  import Sidebar from './lib/Sidebar.svelte';
  import LogPanel from './lib/LogPanel.svelte';
  import ConfirmPanel from './lib/ConfirmPanel.svelte';
  import Home from './lib/Home.svelte';
  import History from './lib/History.svelte';
  import Settings from './lib/Settings.svelte';
  import Setup from './lib/Setup.svelte';
  let width=$state(1120); let height=$state(780); let scale=$state(1); let sidebarWidth=$state(248); let drawer=$state(false); let theme=$state('dark'); let resizing=$state(false);
  let mainEl=$state(null); let menuButton=$state(null); let drawerEl=$state(null); let resizeCleanup;
  // A build that has never been told about this computer asks before it pretends to watch a
  // folder that does not exist. Until that answer arrives the wizard is not shown either way,
  // so a configured installation never flashes a setup screen on the way to the workspace.
  let setupNeeded=$state(null);
  async function checkSetup(){try{const state=await invoke('setup_state');setupNeeded=!state.ready;}catch(e){setupNeeded=false;$actionError='Could not check this computer’s setup: '+String(e);}}
  const compact=$derived(width/scale<820);
  $effect(()=>{if(!compact)drawer=false;});
  const sidebarSize=$derived(clampSidebar(sidebarWidth,width,scale));
  function save(key,value){try{localStorage.setItem(key,String(value));}catch{ $actionNotice='Appearance changes apply now but could not be saved for next time.'; }}
  function setScale(value){scale=normalizeScale(value);save('guide-watcher-scale-v2',scale);}
  function setTheme(value){theme=['dark','light','system'].includes(value)?value:'dark';save('guide-watcher-theme',theme);}
  function resetLayout(){sidebarWidth=248;save('guide-watcher-sidebar-width',248);setScale(1);}
  async function closeDrawer(){const wasOpen=drawer;drawer=false;await tick();if(wasOpen)menuButton?.focus();}
  function navigate(view){$showConfirmPanel=false;$currentView=view;closeDrawer();}
  function keydown(e){
    // A native modal owns keyboard input until it is dismissed.
    if(document.querySelector('dialog[open]')) return;
    if(drawer && compact && e.key==='Tab' && drawerEl){const focusable=[...drawerEl.querySelectorAll('button:not(:disabled),a[href],input,select,[tabindex="0"]')];const first=focusable[0],last=focusable.at(-1);if(e.shiftKey&&document.activeElement===first){e.preventDefault();last?.focus();}else if(!e.shiftKey&&document.activeElement===last){e.preventDefault();first?.focus();}}

    if((e.ctrlKey||e.metaKey)&&!e.altKey){
      if(['+','=','-','0'].includes(e.key)){e.preventDefault();setScale(e.key==='0'?1:scale+(e.key==='-'?-.1:.1));}
      else if(e.key.toLowerCase()==='h'){e.preventDefault();navigate('history');}
      else if(e.key.toLowerCase()==='n'){e.preventDefault();chooseSources('pick');}
    }
    if(e.key==='Escape'&&drawer){e.preventDefault();closeDrawer();}
  }
  function startResize(event){
    event.preventDefault();resizeCleanup?.();resizing=true;
    const start=event.clientX;const original=sidebarSize;
    function move(e){sidebarWidth=clampSidebar(original+(e.clientX-start)/scale,width,scale);}
    function stop(){resizing=false;window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',stop);window.removeEventListener('pointercancel',stop);window.removeEventListener('blur',stop);save('guide-watcher-sidebar-width',sidebarWidth);resizeCleanup=null;}
    resizeCleanup=stop;window.addEventListener('pointermove',move);window.addEventListener('pointerup',stop);window.addEventListener('pointercancel',stop);window.addEventListener('blur',stop);
  }
  function resizeKey(e){if(['ArrowLeft','ArrowRight','Home','End'].includes(e.key)){e.preventDefault();sidebarWidth=clampSidebar(e.key==='Home'?200:e.key==='End'?400:sidebarSize+(e.key==='ArrowLeft'?-16:16),width,scale);save('guide-watcher-sidebar-width',sidebarWidth);}}
   $effect(()=>{if(drawer && drawerEl){queueMicrotask(()=>drawerEl?.querySelector('button')?.focus());}});
  let previousJob=null;
  $effect(()=>{if($selectedJobId && $selectedJobId!==previousJob){previousJob=$selectedJobId;$currentView='run';}});
  $effect(()=>{const view=$currentView;const confirm=$showConfirmPanel; if(mainEl){queueMicrotask(()=>mainEl?.focus({preventScroll:true}));}});
  $effect(()=>{const selected=theme;const media=window.matchMedia('(prefers-color-scheme: light)');function apply(){document.documentElement.dataset.theme=selected==='system'?(media.matches?'light':'dark'):selected;}apply();media.addEventListener('change',apply);return()=>media.removeEventListener('change',apply);});
  onMount(()=>{
    checkSetup();
    try{scale=normalizeScale(localStorage.getItem('guide-watcher-scale-v2'));sidebarWidth=clampSidebar(Number(localStorage.getItem('guide-watcher-sidebar-width'))||248,window.innerWidth,scale);theme=localStorage.getItem('guide-watcher-theme')||'dark';}catch{}
    let disposed=false;const unlisteners=[];
    async function register(name,handler){try{const off=await listen(name,handler);if(disposed)off();else unlisteners.push(off);}catch(e){$actionError='Could not connect to live app updates: '+String(e);}}
    const subscriptions=[
      register('new-files',event=>{
        const modalOpen=!!document.querySelector('dialog[open]');
        receiveWatcherFiles(event.payload,{reveal:!modalOpen});
        if(modalOpen&&event.payload?.length)$actionNotice='New source files are ready to review in Workspace.';
      }),
      register('history-error',event=>{$actionError=String(event.payload);}),
      register('provider-auth',event=>applyProviderAuthEvent(event.payload)),
      register('job-started',event=>updateJobStatus(event.payload.job_id,'working')),
      register('job-output',event=>{const {job_id,line}=event.payload;for(const parsed of parseLineEvents(line)){appendStep(job_id,parsed);if(parsed.activity)updateJobActivity(job_id,parsed.activity);}}),
      register('job-done',event=>{const {job_id,status}=event.payload;if(status)updateJobStatus(job_id,status,Date.now());refreshHistory();}),
    ];
    Promise.all(subscriptions).then(()=>{if(!disposed)refreshHistory();});
    const timer=setInterval(()=>refreshHistory(),5000);
    return()=>{disposed=true;unlisteners.forEach(off=>off());clearInterval(timer);resizeCleanup?.();};
  });
</script>
<svelte:window bind:innerWidth={width} bind:innerHeight={height} onkeydown={keydown}/>
{#if setupNeeded}<Setup onready={()=>{setupNeeded=false;}}/>{:else}
<div class="app-shell" class:resizing style:zoom={scale} style:width={width/scale+'px'} style:height={height/scale+'px'}>
  <a class="skip-link" href="#main-content">Skip to main content</a>
  {#if !compact}<div class="sidebar-container" style:width={sidebarSize+'px'}><Sidebar onnavigate={closeDrawer}/></div><button class="resize-handle" aria-label="Resize sidebar" title="Drag to resize, or use Left and Right arrow keys" onpointerdown={startResize} onkeydown={resizeKey}></button>{/if}
  <div class="workspace" inert={compact && drawer}>
    <header class="toolbar"><div class="toolbar-left">{#if compact}<button bind:this={menuButton} class="button ghost menu-toggle" aria-label="Open navigation" aria-expanded={drawer} onclick={()=>drawer=!drawer}>☰</button>{/if}<span class="location">{$showConfirmPanel?'Review sources':({home:'Workspace',history:'History','full-history':'Full history',run:'Guide activity',settings:'Settings & help'})[$currentView]}</span></div><div class="zoom-controls" aria-label="Interface size"><button aria-label="Zoom out" title="Zoom out (Ctrl −)" disabled={scale<=.85} onclick={()=>setScale(scale-.1)}>−</button><button class="zoom-reset" aria-label="Reset zoom to 100 percent" title="Reset zoom (Ctrl 0)" onclick={()=>setScale(1)}>{Math.round(scale*100)}%</button><button aria-label="Zoom in" title="Zoom in (Ctrl +)" disabled={scale>=2} onclick={()=>setScale(scale+.1)}>+</button></div></header>
    {#if $actionError}<div class="app-message error-box" role="alert"><div><strong>{readableError($actionError)}</strong><details><summary>Technical details</summary><pre>{$actionError}</pre></details></div><button class="button ghost" aria-label="Dismiss error" onclick={()=>{$actionError='';}}>×</button></div>{/if}
    {#if $actionNotice}<div class="app-message notice" role="status"><p>{$actionNotice}</p><button class="button ghost" aria-label="Dismiss notification" onclick={()=>{$actionNotice='';}}>×</button></div>{/if}
    <main id="main-content" bind:this={mainEl} tabindex="-1">
      {#if $showConfirmPanel && $pendingFiles.length}<ConfirmPanel/>{:else if $currentView==='history'}<History/>{:else if $currentView==='full-history'}<History full/>{:else if $currentView==='settings'}<Settings {scale} {setScale} {theme} {setTheme} {resetLayout}/>{:else if $currentView==='run'}<LogPanel/>{:else}<Home/>{/if}
    </main>
  </div>
  {#if compact && drawer}<div class="drawer-layer"><button class="scrim" aria-label="Close navigation" onclick={closeDrawer}></button><div class="drawer" bind:this={drawerEl} role="dialog" aria-modal="true" aria-label="Navigation" tabindex="-1"><div class="drawer-close"><button class="button ghost" onclick={()=>{drawer=false;menuButton?.focus();}}>Close navigation ×</button></div><Sidebar onnavigate={closeDrawer}/></div></div>{/if}
</div>
{/if}
<style>
.app-shell{display:flex;overflow:hidden;background:var(--bg-base);position:relative;}.sidebar-container{flex-shrink:0;min-height:0;container-type:size;}.workspace{display:flex;flex-direction:column;flex:1;min-width:0;min-height:0;}.toolbar{min-height:3.75rem;flex-shrink:0;display:flex;justify-content:space-between;align-items:center;gap:.5rem;padding:.5rem 1.5rem;border-bottom:1px solid var(--border-panel);}.toolbar-left{display:flex;gap:.5rem;align-items:center;min-width:0;}.location{font-size:.9rem;color:var(--text-secondary);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}.zoom-controls{display:flex;align-items:center;gap:.15rem;flex-shrink:0;}.zoom-controls button{min-width:2rem;min-height:2rem;border-radius:5px;font-size:1.1rem;}.zoom-controls button:hover{background:var(--bg-surface-hover);}.zoom-controls .zoom-reset{font-size:.8rem;min-width:3rem;color:var(--text-secondary);font-variant-numeric:tabular-nums;}main{container-type:size;flex:1;min-height:0;min-width:0;overflow:hidden;display:flex;flex-direction:column;}main:focus{outline:none;}.resize-handle{width:5px;border-left:1px solid var(--border-panel);flex-shrink:0;cursor:col-resize;touch-action:none;}.resize-handle:hover,.resizing .resize-handle{background:var(--border-active);}.resizing{cursor:col-resize;user-select:none;}.app-message{display:flex;justify-content:space-between;align-items:flex-start;gap:.5rem;margin:.7rem 1rem 0;max-height:30%;overflow:auto;flex-shrink:0;}.app-message>div{min-width:0;}.notice{padding:.6rem .9rem;background:var(--bg-active);border-radius:6px;}.app-message .button{min-height:1.8rem;}.drawer-layer{position:absolute;inset:0;z-index:50;display:flex;}.scrim{position:absolute;inset:0;background:#0008;}.drawer{container-type:size;position:relative;width:min(290px,90%);height:100%;display:flex;flex-direction:column;background:var(--bg-sidebar);box-shadow:8px 0 40px #0003;}.drawer :global(.sidebar){flex:1;min-height:0;}.drawer-close{display:flex;justify-content:flex-end;padding:.5rem;}.skip-link{position:absolute;top:-100px;left:1rem;z-index:100;padding:.7rem;background:var(--text-primary);color:var(--bg-base);}.skip-link:focus{top:.5rem;}@media(max-width:500px){.toolbar{padding:.4rem .65rem;}.toolbar-left .button{padding:.4rem;}.location{font-size:.85rem;}}
</style>
