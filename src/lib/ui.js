export const MIN_SCALE = 0.85;
export const MAX_SCALE = 2;
export function normalizeScale(value) {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? Math.min(MAX_SCALE, Math.max(MIN_SCALE, Math.round(n * 20) / 20)) : 1;
}
export function clampSidebar(width, viewport, scale = 1) {
  const maximum = Math.max(200, Math.min(400, viewport / scale - 360));
  const n = Number(width);
  return Math.min(maximum, Math.max(200, Number.isFinite(n) ? n : 248));
}
export function filename(path = '') { return path.replaceAll('\\', '/').split('/').pop() || 'Untitled guide'; }
export function folder(path = '') { return path.replaceAll('\\', '/').split('/').slice(-2, -1)[0] || ''; }
export function statusLabel(status) {
  return ({starting:'Preparing',queued:'Queued',working:'In progress',done:'Succeeded','done-warnings':'Succeeded with warnings',failed:'Failed',cancelled:'Cancelled',interrupted:'Interrupted',blocked:'Blocked'})[status] || 'Unknown';
}
export function isActive(status) { return ['starting', 'queued', 'working'].includes(status); }
export function duration(start, end = Date.now()) {
  if (!start) return 'Not started';
  const s = Math.max(0, Math.floor(((end ?? Date.now()) - start) / 1000));
  return s >= 3600 ? Math.floor(s/3600)+'h '+Math.floor(s%3600/60)+'m' : s >= 60 ? Math.floor(s/60)+'m '+s%60+'s' : s+'s';
}
export function dateTime(value) { return value ? new Date(value).toLocaleString([], { dateStyle:'medium', timeStyle:'short' }) : 'Not recorded'; }
export function readableError(error) {
  const detail = String(error?.message || error || 'Unknown error');
  if (/permission|access.denied/i.test(detail)) return 'Access was denied. Check that the source folder is readable and the output folder is writable.';
  if (/auth|login|sign.in|401/i.test(detail)) return 'The provider needs you to sign in again. Open its CLI, sign in, then retry this run.';
  if (/quota|rate.limit|429|capacity/i.test(detail)) return 'The provider is temporarily unavailable or has reached its usage limit. Try again when access is available.';
  if (/not.found|does not exist|no such file/i.test(detail)) return 'A required file or program could not be found. Check the source path and provider installation.';
  if (/timeout|timed.out/i.test(detail)) return 'The operation took too long. Check your connection and try again.';
  return 'The operation could not finish. The details below explain what happened.';
}

// The four steps a guide goes through, for the plain progress view. The app's phase messages
// are written for people, so the step is derived from those same words rather than from a
// second source of truth that could drift away from them.
export const RUN_STEPS = [
  { id:'read', label:'Read' },
  { id:'research', label:'Research' },
  { id:'write', label:'Write' },
  { id:'check', label:'Check' },
];
export function runStep(message = '') {
  const text = String(message).toLowerCase();
  if (/checking the guide|verif|repair|is ready|published/.test(text)) return 3;
  if (/writing the guide|guide draft|deepening pass|pedagogy pass|writer finished|continuing the draft|picking up the|style plan/.test(text)) return 2;
  if (/research|assembling the source|freezing copies|starting the writer|figure|visual/.test(text)) return 1;
  return 0;
}
// Minutes left, from the app's own estimate and when it was given. Returns null when the app
// has not estimated this phase or the estimate has run out: a countdown is only shown while it
// still means something.
export function minutesLeft(events = [], now = Date.now()) {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    const match = /about (\d+) minutes/i.exec(String(event?.message || ''));
    if (!match) continue;
    const started = Number(event?.timestamp);
    if (!Number.isFinite(started)) return null;
    const left = Math.round(Number(match[1]) - (now - started) / 60000);
    return left > 0 ? left : null;
  }
  return null;
}
