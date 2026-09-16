import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { pendingFiles,pendingSource,showConfirmPanel,appendPendingFiles,deferWatcherFiles,revealDeferredWatcherFiles } from '../stores/jobs.js';
import { currentView,actionError,actionNotice,busyAction } from '../stores/navigation.js';
export async function chooseSources(kind) {
  if (get(busyAction)) return;
  busyAction.set(kind); actionError.set(''); actionNotice.set('');
  try {
    if (kind === 'resume') {
      if (get(pendingFiles).length) { actionNotice.set('Review or dismiss your selected sources before opening a saved prep file.'); currentView.set('home'); showConfirmPanel.set(true); return; }
      const path = await invoke('pick_prep_file');
      if (!path) { revealDeferredWatcherFiles(); return; }
      deferWatcherFiles(get(pendingFiles));
      pendingSource.set('resume'); pendingFiles.set([path]);
    } else {
      if (get(pendingSource) === 'resume' && get(pendingFiles).length) { actionNotice.set('Finish or dismiss your saved prep selection before adding source files.'); showConfirmPanel.set(true); return; }
      const files = await invoke(kind === 'scan' ? 'scan_watch_folder' : 'pick_files');
      if (!files?.length) { if(kind === 'scan') actionNotice.set('No new supported source files were found in the configured semester folder. Existing guides are skipped.'); return; }
      pendingSource.set('scan'); appendPendingFiles(files);
    }
    currentView.set('home'); showConfirmPanel.set(true);
  } catch(error) { actionError.set(String(error)); if(kind==='resume' && get(pendingSource)!=='resume')revealDeferredWatcherFiles(); }
  finally { busyAction.set(''); }
}
