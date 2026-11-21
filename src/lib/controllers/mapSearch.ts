// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Map search: search → hits → jump (UI: MapSearch.svelte). Coordinates are parsed locally and never leave the
// device; anything else goes to the backend's place search (Nominatim, `map_search_places`) on an explicit
// search only (usage policy). A jump is a map-focus request (stores/mapFocus).

import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { locale, t } from 'svelte-i18n';
import { parseCoordinates } from '$lib/helpers/coordinateParse';
import { requestMapFocus } from '$lib/stores/mapFocus';
import { gcsLocation } from '$lib/stores/gcsLocation';
import { userGeoLocation, locateUserOnce } from '$lib/helpers/userLocation';
import { pushLocalStatus } from '$lib/stores/statusText';
import type { GeoBounds } from '$lib/stores/customMaps';

/** One place hit (backend `PlaceHit`). */
export interface PlaceHit {
  name: string;
  lat: number;
  lon: number;
  /** `[west, south, east, north]`, or null. */
  bbox: GeoBounds | null;
  kind: string | null;
}

/** What a search ended in. `jumped` = coordinates or exactly one hit — the map is already on its way. */
export type SearchOutcome =
  | { kind: 'jumped' }
  | { kind: 'hits'; hits: PlaceHit[] }
  | { kind: 'none' }
  | { kind: 'offline' }
  | { kind: 'refused' }
  | { kind: 'failed' };

/** Error tokens of `map_search_places` (src-tauri flightlog/geocode.rs `SearchError::token`). */
const ERROR_OUTCOMES: Record<string, SearchOutcome> = {
  offline: { kind: 'offline' },
  refused: { kind: 'refused' },
  empty_query: { kind: 'none' },
};

/** STATUSTEXT warning severity — passes the default "Warning" toast filter. */
const SEVERITY_WARNING = 4;

/** Jump the map views to a hit: its box when it has one, else the point. */
export function jumpToHit(hit: PlaceHit): void {
  requestMapFocus({ source: 'search', bounds: hit.bbox, lat: hit.lat, lon: hit.lon });
}

/** Digits with only separators, degree marks, signs, brackets and single hemisphere letters — no word (`Essen 1`
 *  and `B 100` are not). No lookbehind: iOS WebKit before 16.4 cannot parse one. */
function looksLikeCoordinates(text: string): boolean {
  return /\d/.test(text) && /^(?:[\d\s.,;:°º'"′″+\-()[\]|/]|[NSEWnsew](?![A-Za-z]))+$/.test(text);
}

function jumpToPoint(lat: number, lon: number): void {
  requestMapFocus({ source: 'search', bounds: null, lat, lon });
}

/** Search the text: coordinates jump at once (offline, nothing sent); a place name asks the
 *  backend. Exactly one hit jumps as well — unless `isCurrent` says the search was given up while
 *  the answer was on its way (the line closed, a newer search). */
export async function runMapSearch(text: string, isCurrent: () => boolean): Promise<SearchOutcome> {
  const query = text.trim();
  if (!query) return { kind: 'none' };
  const coords = parseCoordinates(query);
  if (coords) {
    jumpToPoint(coords.lat, coords.lon);
    return { kind: 'jumped' };
  }
  // Looks like coordinates but did not parse (out of range, a stray character, a date): never send
  // that to the place search — coordinates stay on the device, and the service would answer such a
  // text with a reverse lookup the pilot did not ask for.
  if (looksLikeCoordinates(query)) return { kind: 'none' };
  try {
    const hits = await invoke<PlaceHit[]>('map_search_places', { query, lang: get(locale) ?? 'en' });
    if (hits.length === 0) return { kind: 'none' };
    if (hits.length === 1) {
      if (isCurrent()) jumpToHit(hits[0]);
      return { kind: 'jumped' };
    }
    return { kind: 'hits', hits };
  } catch (e) {
    const token = typeof e === 'string' ? e : '';
    // Status only — never the query (it can be someone's address).
    console.warn(`[map-search] search failed: ${token || 'unexpected error'}`);
    return ERROR_OUTCOMES[token] ?? { kind: 'failed' };
  }
}

/** "To my location": the known ground-station position (GCS marker, else the last OS fix), else one OS request;
 *  the GCS marker setting stays. false = no position (notice in the toast banner; `noticeShown` false when the
 *  filter hid it). A late OS answer after `isCurrent` turned false neither jumps nor notifies. */
export async function goToMyLocation(isCurrent: () => boolean): Promise<{ ok: boolean; noticeShown: boolean }> {
  const known = get(gcsLocation) ?? get(userGeoLocation);
  if (known) {
    jumpToPoint(known.lat, known.lon);
    return { ok: true, noticeShown: false };
  }
  const fix = await locateUserOnce();
  if (!isCurrent()) return { ok: !!fix, noticeShown: false };
  if (fix) {
    jumpToPoint(fix.lat, fix.lon);
    return { ok: true, noticeShown: false };
  }
  return { ok: false, noticeShown: pushLocalStatus(SEVERITY_WARNING, get(t)('mapSearch.locationFailed')) };
}
