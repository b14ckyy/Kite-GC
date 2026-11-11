// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Eight-point compass rose as i18n keys, so a bearing reads "NO" in German or "SO" in Spanish
// instead of a hard-coded English "NE" / "SW".

const POINTS = ['n', 'ne', 'e', 'se', 's', 'sw', 'w', 'nw'] as const;

/** i18n key (`compass.*`) of the compass point nearest to a bearing in degrees. */
export function compassPointKey(deg: number): `compass.${(typeof POINTS)[number]}` {
  const i = ((Math.round(deg / 45) % 8) + 8) % 8;
  return `compass.${POINTS[i]}`;
}
