import test from 'node:test';import assert from 'node:assert/strict';
import {emptySubject,defaultFolderFor,normalizeSubjects,draftProblems,blockingRequirements,lectureFilesProblem} from './setupFlow.js';

test('a subject keeps only what the person actually typed', () => {
  const subjects = normalizeSubjects([
    { label: '  Computer Networks  ', folder: '  ', lectureFiles: ' ^chapter ' },
    { label: '   ', folder: 'Ignored' },
    { label: 'Anatomy', folder: 'Year 2/Anatomy' },
    undefined,
  ]);
  assert.deepEqual(subjects, [
    { label: 'Computer Networks', folder: 'Computer Networks', lectureFiles: '^chapter' },
    { label: 'Anatomy', folder: 'Year 2/Anatomy', lectureFiles: '' },
  ]);
  // A blank row is a row waiting to be filled in, never a subject.
  assert.deepEqual(normalizeSubjects([emptySubject()]), []);
  assert.equal(defaultFolderFor('  Circuit Lab '), 'Circuit Lab');
});

test('the wizard says what is still missing before anything is sent', () => {
  assert.deepEqual(draftProblems({ watchDir: '', automationDir: '', subjects: [] }), [
    'Choose the folder that holds your course material.',
    'Choose the _automation folder that came with Guide Watcher.',
    'Add at least one subject.',
  ]);
  const complete = {
    watchDir: 'C:/Study',
    automationDir: 'C:/Study/_automation',
    subjects: [{ label: 'Photography', folder: 'Photography' }],
  };
  assert.deepEqual(draftProblems(complete), []);
  // Two subjects with one name would write two settings entries with the same derived id.
  const clashing = { ...complete, subjects: [{ label: 'Photography' }, { label: 'photography' }] };
  assert.deepEqual(draftProblems(clashing), ['Two subjects are both called “photography”.']);
  // The file rule fails open by design, so an unusable pattern would take in every file.
  const broken = { ...complete, subjects: [{ label: 'Photography', lectureFiles: '^lesson[' }] };
  assert.match(draftProblems(broken)[0], /^Photography: That file-name pattern is not valid/);
});

test('only the checks that really block are put in front of the person', () => {
  const state = { requirements: [
    { id: 'claude', required: true, satisfied: false },
    { id: 'codex', required: true, satisfied: true },
    { id: 'archive', required: false, satisfied: false },
  ] };
  assert.deepEqual(blockingRequirements(state).map(item => item.id), ['claude']);
  assert.deepEqual(blockingRequirements(undefined), []);
});

test('an unusable file-name pattern is caught while it is being typed', () => {
  assert.equal(lectureFilesProblem(''), '');
  assert.equal(lectureFilesProblem('  '), '');
  assert.equal(lectureFilesProblem('^lecture[ _-]?\\d+'), '');
  assert.match(lectureFilesProblem('^lecture['), /not valid/);
});
