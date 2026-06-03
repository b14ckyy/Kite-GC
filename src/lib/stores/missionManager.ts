// Mission Manager view state — lifted out of the panel component so it survives the
// open/close (the same mission stays selected on reopen) and so +page can size the
// nav-panel like the logbook (wide list, widest when a mission is selected).
// TODO: other panels (UAV info, settings sub-views) should likewise persist their view
// state on close/reopen — see MEMORY note.

import { writable } from 'svelte/store';

/** Whether the mission planner panel is showing the Mission Manager view. */
export const missionManagerOpen = writable<boolean>(false);

/** The library mission selected in the Manager (persists across close/reopen). */
export const missionManagerSelectedId = writable<number | null>(null);

/** Set to a flight id to request jumping to it in the Logbook (e.g. from the Manager's
 *  "flights with this mission" list). `+page` watches this, switches to the logbook tab,
 *  selects the flight, and resets it to null. */
export const requestOpenFlightId = writable<number | null>(null);
