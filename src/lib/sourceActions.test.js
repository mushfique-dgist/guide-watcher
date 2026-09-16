import test, { beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { get } from 'svelte/store';
import { chooseSources } from './sourceActions.js';
import { pendingFiles, pendingSource, showConfirmPanel, deferredWatcherFiles, receiveWatcherFiles, revealDeferredWatcherFiles } from '../stores/jobs.js';
import { busyAction, actionError, actionNotice, currentView } from '../stores/navigation.js';

let originalWindow;
let calls;
let resolvePicker;
let rejectPicker;
beforeEach(() => {
  originalWindow = globalThis.window;
  calls = [];
  pendingFiles.set([]);
  pendingSource.set('watcher');
  showConfirmPanel.set(false);
  deferredWatcherFiles.set([]);
  busyAction.set('');
  actionError.set('');
  actionNotice.set('');
  currentView.set('home');
  globalThis.window = { __TAURI_INTERNALS__: { invoke(command) {
    calls.push(command);
    return new Promise((resolve, reject) => { resolvePicker = resolve; rejectPicker = reject; });
  } } };
});
afterEach(() => {
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
});

const prep = 'C:/Courses/Week_2_Guide.prep.md';
const source = 'C:/Courses/Week_3.pdf';

test('successful prep picker defers all sources received while it was open', async () => {
  const operation = chooseSources('resume');
  assert.deepEqual(calls, ['pick_prep_file']);
  receiveWatcherFiles([source]);
  receiveWatcherFiles(['c:\\courses\\WEEK_3.pdf', 'C:/Courses/Week_4.pdf']);
  resolvePicker(prep);
  await operation;
  assert.deepEqual(get(pendingFiles), [prep]);
  assert.equal(get(pendingSource), 'resume');
  assert.equal(get(showConfirmPanel), true);
  assert.deepEqual(get(deferredWatcherFiles), [source, 'C:/Courses/Week_4.pdf']);
  receiveWatcherFiles(['C:/Courses/Week_5.pdf']);
  pendingFiles.set([]);
  showConfirmPanel.set(false);
  revealDeferredWatcherFiles();
  assert.deepEqual(get(pendingFiles), [source, 'C:/Courses/Week_4.pdf', 'C:/Courses/Week_5.pdf']);
  assert.deepEqual(get(deferredWatcherFiles), []);
  assert.equal(get(pendingSource), 'watcher');
  assert.equal(get(busyAction), '');
});

test('picker success preserves discoveries even after navigating away from their review', async () => {
  deferredWatcherFiles.set([source]);
  const operation = chooseSources('resume');
  receiveWatcherFiles([source, 'C:/Courses/Week_4.pdf']);
  showConfirmPanel.set(false);
  currentView.set('history');
  resolvePicker(prep);
  await operation;
  assert.deepEqual(get(pendingFiles), [prep]);
  assert.deepEqual(get(deferredWatcherFiles), [source, 'C:/Courses/Week_4.pdf']);
  assert.equal(get(currentView), 'home');
});

for (const outcome of ['cancel', 'error']) {
  test('picker ' + outcome + ' retains watcher discoveries and allows a later selection', async () => {
    const operation = chooseSources('resume');
    receiveWatcherFiles([source]);
    if (outcome === 'cancel') resolvePicker(null);
    else rejectPicker(new Error('Picker unavailable'));
    await operation;
    assert.deepEqual(get(pendingFiles), [source]);
    assert.deepEqual(get(deferredWatcherFiles), []);
    assert.equal(get(pendingSource), 'watcher');
    assert.equal(get(showConfirmPanel), true);
    assert.equal(get(busyAction), '');
    assert.equal(get(actionError), outcome === 'error' ? 'Error: Picker unavailable' : '');
    pendingFiles.set([]);
    showConfirmPanel.set(false);
    const next = chooseSources('resume');
    resolvePicker(prep);
    await next;
    assert.deepEqual(get(pendingFiles), [prep]);
    assert.equal(get(actionError), '');
  });
}

test('existing selections prevent the picker from opening', async () => {
  pendingFiles.set([source]);
  await chooseSources('resume');
  assert.deepEqual(calls, []);
  assert.deepEqual(get(pendingFiles), [source]);
  assert.equal(get(showConfirmPanel), true);
  assert.equal(get(busyAction), '');
});

test('concurrent source actions do not open a second picker', async () => {
  const operation = chooseSources('resume');
  await chooseSources('pick');
  await chooseSources('resume');
  assert.deepEqual(calls, ['pick_prep_file']);
  resolvePicker(prep);
  await operation;
  assert.deepEqual(get(pendingFiles), [prep]);
  assert.deepEqual(get(deferredWatcherFiles), []);
});

test('navigating away from a saved prep selection does not mix watcher sources into it',()=>{pendingSource.set('resume');pendingFiles.set([prep]);showConfirmPanel.set(false);receiveWatcherFiles([source]);assert.deepEqual(get(pendingFiles),[prep]);assert.deepEqual(get(deferredWatcherFiles),[source]);});
