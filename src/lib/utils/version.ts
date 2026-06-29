// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Pragmatic semver-ish comparison for the update check. Handles the project's tag scheme
// (`1.0.0`, `1.0.0-b1`, `1.0.0-b2`, `1.1.0-rc1`, …): numeric MAJOR.MINOR.PATCH core, then a
// prerelease suffix where a *stable* (no suffix) version outranks a prerelease of the same core,
// and prereleases compare naturally (so `b2` < `b10`). Not a full SemVer 2.0 implementation — just
// enough for our controlled, monotonic release tags.

function parse(v: string): { nums: [number, number, number]; pre: string } {
  const s = v.trim().replace(/^v/i, '');
  const dash = s.indexOf('-');
  const core = dash === -1 ? s : s.slice(0, dash);
  const pre = dash === -1 ? '' : s.slice(dash + 1);
  const parts = core.split('.').map((n) => parseInt(n, 10) || 0);
  return { nums: [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0], pre };
}

/** Natural comparison of two prerelease suffixes (numeric chunks compared as numbers). */
function comparePre(a: string, b: string): number {
  const ra = a.match(/\d+|\D+/g) ?? [];
  const rb = b.match(/\d+|\D+/g) ?? [];
  const n = Math.max(ra.length, rb.length);
  for (let i = 0; i < n; i++) {
    const sa = ra[i] ?? '';
    const sb = rb[i] ?? '';
    if (sa === sb) continue;
    const na = /^\d+$/.test(sa);
    const nb = /^\d+$/.test(sb);
    if (na && nb) {
      const d = parseInt(sa, 10) - parseInt(sb, 10);
      if (d !== 0) return d < 0 ? -1 : 1;
    } else {
      return sa < sb ? -1 : 1;
    }
  }
  return 0;
}

/** Compare two versions. Returns <0 if `a` < `b`, 0 if equal, >0 if `a` > `b`. A leading `v` is
 *  ignored. A stable version outranks the same-core prerelease (`1.0.0` > `1.0.0-b1`). */
export function compareVersions(a: string, b: string): number {
  const pa = parse(a);
  const pb = parse(b);
  for (let i = 0; i < 3; i++) {
    if (pa.nums[i] !== pb.nums[i]) return pa.nums[i] < pb.nums[i] ? -1 : 1;
  }
  if (pa.pre === pb.pre) return 0;
  if (!pa.pre) return 1; // a is stable, b is a prerelease of the same core → a is newer
  if (!pb.pre) return -1; // a is a prerelease, b is stable → b is newer
  return comparePre(pa.pre, pb.pre);
}
