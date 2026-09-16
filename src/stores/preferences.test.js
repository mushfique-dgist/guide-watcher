import test, { beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { get } from 'svelte/store';
import { POWER_MODE_KEY, readPowerMode, powerMode, setPowerMode } from './preferences.js';

let store;
beforeEach(() => {
  store = new Map();
  globalThis.localStorage = {
    getItem: key => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, String(value)),
    removeItem: key => store.delete(key),
  };
  setPowerMode(false);
});

test('power mode is off until it is turned on, and then it stays on', () => {
  assert.equal(readPowerMode(), false);
  assert.equal(get(powerMode), false);

  setPowerMode(true);
  assert.equal(get(powerMode), true);
  assert.equal(store.get(POWER_MODE_KEY), 'on');
  // A fresh read of storage, as the next launch would do.
  assert.equal(readPowerMode(), true);

  setPowerMode(false);
  assert.equal(get(powerMode), false);
  assert.equal(readPowerMode(), false);
});

test('a value that is not the stored word is off, and locked storage never breaks the toggle', () => {
  store.set(POWER_MODE_KEY, 'yes');
  assert.equal(readPowerMode(), false);
  store.set(POWER_MODE_KEY, 'ON');
  assert.equal(readPowerMode(), false);

  globalThis.localStorage = {
    getItem() { throw new Error('storage is blocked'); },
    setItem() { throw new Error('storage is blocked'); },
  };
  assert.equal(readPowerMode(), false);
  assert.equal(setPowerMode(true), true);
  assert.equal(get(powerMode), true, 'the switch still works for this session');

  globalThis.localStorage = undefined;
  assert.equal(readPowerMode(), false);
  assert.equal(setPowerMode(false), false);
});
