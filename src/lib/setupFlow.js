// What the first-run wizard knows without asking the backend.
//
// The backend has the final word: it checks every answer against the machine and refuses a setup
// that would fail later. These helpers exist so the wizard can answer the person immediately,
// while they are still typing, rather than only after a round trip.

/** A blank subject row, so the wizard never starts with an empty list. */
export function emptySubject() {
  return { label: '', folder: '', lectureFiles: '' };
}

/**
 * The folder a subject gets when the person has not chosen one. Using the name they just typed is
 * right far more often than an empty box, and it stays relative to the study folder.
 */
export function defaultFolderFor(label) {
  return String(label ?? '').trim();
}

/** Subjects as the backend wants them: named, trimmed, and each with a folder. */
export function normalizeSubjects(subjects) {
  return (subjects ?? [])
    .map(subject => ({
      label: String(subject?.label ?? '').trim(),
      folder: String(subject?.folder ?? '').trim(),
      lectureFiles: String(subject?.lectureFiles ?? '').trim(),
    }))
    .filter(subject => subject.label !== '')
    .map(subject => ({ ...subject, folder: subject.folder || defaultFolderFor(subject.label) }));
}

/**
 * Everything standing between this draft and a working setup, in the words the wizard shows.
 * An empty list means the answers are complete enough to send.
 */
export function draftProblems(draft) {
  const problems = [];
  if (!String(draft?.watchDir ?? '').trim()) problems.push('Choose the folder that holds your course material.');
  if (!String(draft?.automationDir ?? '').trim()) problems.push('Choose the _automation folder that came with Guide Watcher.');
  const subjects = normalizeSubjects(draft?.subjects);
  if (subjects.length === 0) problems.push('Add at least one subject.');
  const seen = new Set();
  for (const subject of subjects) {
    const key = subject.label.toLowerCase();
    if (seen.has(key)) problems.push(`Two subjects are both called “${subject.label}”.`);
    seen.add(key);
  }
  return problems;
}

/** The checks the person still has to act on, worst first: what blocks, then what is merely nice. */
export function blockingRequirements(state) {
  return (state?.requirements ?? []).filter(item => item.required && !item.satisfied);
}

/** A regular expression the person typed is theirs, but an invalid one silently matches nothing. */
export function lectureFilesProblem(pattern) {
  const value = String(pattern ?? '').trim();
  if (!value) return '';
  try {
    new RegExp(value, 'i');
    return '';
  } catch (error) {
    return `That file-name pattern is not valid: ${error.message}`;
  }
}
