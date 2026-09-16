import test from 'node:test';import assert from 'node:assert/strict';import {normalizeScale,clampSidebar,statusLabel,duration,filename,runStep,minutesLeft} from './ui.js';
test('stored zoom is bounded and malformed values reset',()=>{for(const input of [undefined,null,'',0,-1,'no',Infinity,NaN])assert.equal(normalizeScale(input),1);assert.equal(normalizeScale(99),2);assert.equal(normalizeScale(.1),.85);assert.equal(normalizeScale(1.234),1.25);});
test('sidebar bounds preserve the workspace at each scale',()=>{for(const width of [600,800,1120,1920])for(const scale of [.85,1,1.5,2])for(const stored of [-100,0,248,400,9999,NaN]){const result=clampSidebar(stored,width,scale);assert(result>=200&&result<=400);}});
test('all recorded outcomes have distinct plain-language labels',()=>{assert.equal(statusLabel('blocked'),'Blocked');assert.equal(statusLabel('interrupted'),'Interrupted');assert.equal(statusLabel('cancelled'),'Cancelled');assert.equal(statusLabel('done'),'Succeeded');assert.equal(statusLabel('unknown'),'Unknown');});
test('durations and filenames tolerate missing data',()=>{assert.equal(duration(null),'Not started');assert.equal(duration(100,99),'0s');assert.equal(filename('C:/A/lecture.pdf'),'lecture.pdf');});

test('the four-step progress reads the same words the app shows the user', () => {
  const step = message => runStep(message);
  assert.equal(step('Checking your selected files and settings.'), 0);
  assert.equal(step('Step 1 of 2 · Extracting the lecture text.'), 0);
  assert.equal(step('Step 1 of 2 · Choosing which figures are worth showing in the guide.'), 1);
  assert.equal(step('Step 1 of 2 · Researching the topic and assembling the source material.'), 1);
  assert.equal(step('Freezing copies of your source files, so the guide is written from exactly these versions.'), 1);
  assert.equal(step('Step 2 of 2 · Writing the guide draft (115 slides).'), 2);
  assert.equal(step('Step 2 of 2 · Deepening pass 1/3 — adding worked examples.'), 2);
  assert.equal(step('The writer finished this pass. Checking the result before it is kept.'), 2);
  assert.equal(step('Checking the guide against the sources.'), 3);
  assert.equal(step('Your guide is ready. It passed every check and was published with its figures.'), 3);
  // Wording the app has never used must not throw or skip ahead.
  assert.equal(step('Some brand new phase'), 0);
  assert.equal(step(), 0);
});

test('the countdown only runs while the app has actually estimated the work', () => {
  const at = minutes => Date.UTC(2026, 8, 14, 12, minutes);
  const events = [
    { timestamp: at(0), message: 'Checking your selected files and settings.' },
    { timestamp: at(2), message: 'Writing the guide draft (115 slides). This pass usually takes about 34 minutes.' },
  ];
  assert.equal(minutesLeft(events, at(12)), 24);
  // Once the estimate is spent, nothing is shown rather than a negative or a stuck zero.
  assert.equal(minutesLeft(events, at(40)), null);
  // The newest estimate wins over an older one.
  const later = [...events, { timestamp: at(36), message: 'Deepening pass 1/3. About 30 minutes.' }];
  assert.equal(minutesLeft(later, at(46)), 20);
  assert.equal(minutesLeft([{ timestamp: at(0), message: 'no estimate here' }], at(5)), null);
  assert.equal(minutesLeft([], at(5)), null);
  assert.equal(minutesLeft([{ message: 'about 10 minutes' }], at(5)), null);
});
