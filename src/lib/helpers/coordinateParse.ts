// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Coordinates typed into the map search, parsed locally (never sent, work offline): decimal degrees (`51.4886,
// 11.97`, decimal comma `51,4886; 11,97`), hemisphere letters before/after, degrees + decimal minutes, and DMS
// (`51°29'19"N 11°58'12"E`). Without letters the first value is the latitude; out-of-range or ambiguous → null.

export interface ParsedCoordinates {
  lat: number;
  lon: number;
}

type Hemisphere = 'N' | 'S' | 'E' | 'W';

type Field =
  | { t: 'num'; v: number; int: boolean; mark: 0 | 1 | 2 | null }
  | { t: 'hemi'; h: Hemisphere }
  | { t: 'sign'; neg: boolean }
  | { t: 'sep' };

interface Component {
  value: number;
  axis: 'lat' | 'lon' | null;
  /** How many numbers (degrees / minutes / seconds) it was written with. */
  parts: number;
  /** Where its hemisphere letter stands, if it has one. */
  hemiAt: 'lead' | 'trail' | null;
}

/** Longer than any sensible coordinate pair — such text is a place name. */
const MAX_LEN = 80;

const MARKS: Record<string, 0 | 1 | 2> = { '°': 0, "'": 1, '"': 2 };

/** The text as fields, or null when it holds anything that cannot be part of a coordinate. */
function tokenize(input: string): Field[] | null {
  let s = input.trim().toUpperCase();
  if (!s || s.length > MAX_LEN) return null;
  // One spelling per mark: primes and typographic quotes, two apostrophes for seconds.
  // Pasted forms: `(51.48, 11.97)`, `[51.48, 11.97]`, a trailing separator.
  s = s.trim().replace(/^[([]\s*/, '').replace(/\s*[)\]]$/, '').replace(/[,;]\s*$/, '');
  // Two dots in one run of digits is a date or a version (`17.06.2024`), not a number.
  if (/\d\.\d+\.\d/.test(s)) return null;
  s = s
    .replace(/[º˚]/g, '°')
    .replace(/[′’‘´`]/g, "'")
    .replace(/[″“”]/g, '"')
    .replace(/''/g, '"');
  // Separators. With a decimal point anywhere, every comma separates; without one, a comma between
  // two digits is a decimal comma (`51,4886 11,97`), any other comma separates. A semicolon always
  // separates.
  s = s.replace(/;/g, '|');
  if (s.includes('.')) s = s.replace(/,/g, '|');
  else s = s.replace(/,(?=\s)/g, '|').replace(/(\d),(\d)/g, '$1.$2').replace(/,/g, '|');

  const fields: Field[] = [];
  const re = /\s+|(\d+(?:\.\d+)?|\.\d+)|([°'"])|([NSEW])|([+-])|(\|)/y;
  let pos = 0;
  while (pos < s.length) {
    re.lastIndex = pos;
    const m = re.exec(s);
    if (!m) return null; // another letter or symbol: not a coordinate
    pos = re.lastIndex;
    if (m[1] !== undefined) {
      fields.push({ t: 'num', v: Number(m[1]), int: !m[1].includes('.'), mark: null });
    } else if (m[2] !== undefined) {
      const prev = fields[fields.length - 1];
      if (!prev || prev.t !== 'num' || prev.mark !== null) return null;
      prev.mark = MARKS[m[2]];
    } else if (m[3] !== undefined) {
      fields.push({ t: 'hemi', h: m[3] as Hemisphere });
    } else if (m[4] !== undefined) {
      fields.push({ t: 'sign', neg: m[4] === '-' });
    } else if (m[5] !== undefined) {
      fields.push({ t: 'sep' });
    }
  }
  return fields;
}

/** One latitude or longitude: [hemisphere | sign] deg [min [sec]] [hemisphere]. */
function component(fs: Field[]): Component | null {
  let hemi: Hemisphere | null = null;
  let hemiAt: Component['hemiAt'] = null;
  let signed = false;
  let neg = false;
  const nums: { v: number; int: boolean; mark: 0 | 1 | 2 | null }[] = [];
  for (let i = 0; i < fs.length; i++) {
    const f = fs[i];
    if (f.t === 'hemi') {
      if (hemi || (i !== 0 && i !== fs.length - 1)) return null;
      hemi = f.h;
      hemiAt = i === 0 ? 'lead' : 'trail';
    } else if (f.t === 'sign') {
      if (signed || nums.length > 0 || fs[i + 1]?.t !== 'num') return null;
      signed = true;
      neg = f.neg;
    } else if (f.t === 'num') {
      nums.push(f);
    } else {
      return null;
    }
  }
  if (nums.length < 1 || nums.length > 3) return null;
  for (let k = 0; k < nums.length; k++) {
    const n = nums[k];
    if (n.mark !== null && n.mark !== k) return null; // a minute mark on the degrees, …
    if (k < nums.length - 1 && !n.int) return null; // only the last part may carry decimals
    if (k > 0 && n.v >= 60) return null;
  }
  // A sign and a hemisphere letter together (`-33.9 S`) are ambiguous.
  if (hemi && signed) return null;
  let value = nums[0].v + (nums[1]?.v ?? 0) / 60 + (nums[2]?.v ?? 0) / 3600;
  if (neg || hemi === 'S' || hemi === 'W') value = -value;
  const axis = hemi === 'N' || hemi === 'S' ? 'lat' : hemi ? 'lon' : null;
  return { value, axis, parts: nums.length, hemiAt };
}

function pair(a: Component, b: Component): ParsedCoordinates | null {
  if (a.axis && a.axis === b.axis) return null; // two latitudes
  const aIsLon = a.axis === 'lon' || b.axis === 'lat';
  const lat = aIsLon ? b.value : a.value;
  const lon = aIsLon ? a.value : b.value;
  if (!Number.isFinite(lat) || !Number.isFinite(lon) || Math.abs(lat) > 90 || Math.abs(lon) > 180) return null;
  return { lat, lon };
}

/** A latitude/longitude pair from free text, or null when the text is not one (see the file header). */
export function parseCoordinates(input: string): ParsedCoordinates | null {
  const fields = tokenize(input);
  if (!fields || fields.length < 2) return null;

  // An explicit separator decides the split.
  const seps = fields.reduce<number[]>((acc, f, i) => (f.t === 'sep' ? [...acc, i] : acc), []);
  if (seps.length > 1) return null;
  if (seps.length === 1) {
    const a = component(fields.slice(0, seps[0]));
    const b = component(fields.slice(seps[0] + 1));
    return a && b ? pair(a, b) : null;
  }

  // Otherwise every split leaving two valid halves is a candidate. Plain numbers must split evenly; a letter
  // between two numbers goes trailing when the text starts with a number, leading when it starts with a letter.
  // Still more than one: the even split, or none.
  const plain = !fields.some((f) => f.t === 'hemi' || (f.t === 'num' && f.mark !== null));
  const style = fields[0].t === 'hemi' ? 'lead' : 'trail';
  let found: { r: ParsedCoordinates; even: boolean; styled: boolean }[] = [];
  for (let i = 1; i < fields.length; i++) {
    const a = component(fields.slice(0, i));
    const b = component(fields.slice(i));
    if (!a || !b) continue;
    const r = pair(a, b);
    if (!r) continue;
    const even = a.parts === b.parts;
    if (plain && !even) continue;
    found.push({ r, even, styled: [a.hemiAt, b.hemiAt].every((h) => h === null || h === style) });
  }
  for (const keep of [(f: (typeof found)[number]) => f.styled, (f: (typeof found)[number]) => f.even]) {
    if (found.length <= 1) break;
    const narrowed = found.filter(keep);
    if (narrowed.length > 0) found = narrowed;
  }
  return found.length === 1 ? found[0].r : null;
}
