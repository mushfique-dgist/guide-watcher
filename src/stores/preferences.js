import { writable } from 'svelte/store';

// Power mode is one switch, kept on this computer. It never changes what the app can do or
// which pipeline runs: it decides how much the daily screens show. Off is the default, so a
// newcomer meets the plain path; a power user turns it on once and keeps today's screens.
export const POWER_MODE_KEY = 'guide-watcher-power-mode';

function storage() {
  try {
    const value = globalThis.localStorage;
    return value && typeof value.getItem === 'function' && typeof value.setItem === 'function' ? value : null;
  } catch { return null; }
}

export function readPowerMode() {
  try { return storage()?.getItem(POWER_MODE_KEY) === 'on'; } catch { return false; }
}

export const powerMode = writable(readPowerMode());

export function setPowerMode(value) {
  const on = Boolean(value);
  powerMode.set(on);
  try { storage()?.setItem(POWER_MODE_KEY, on ? 'on' : 'off'); } catch { /* a locked store must not break the toggle */ }
  return on;
}
