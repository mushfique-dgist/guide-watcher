import { writable } from 'svelte/store';
export const historyReturnView = writable('history');
export const currentView = writable('home');
export const actionError = writable('');
export const actionNotice = writable('');
export const busyAction = writable('');
