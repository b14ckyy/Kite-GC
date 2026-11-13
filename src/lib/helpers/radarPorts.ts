// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Local radar source links (ADS-B receivers, the FormationFlight module): how a source's link is shown,
// edited and pushed on this platform, and which serial port is claimed by whom. The one place for
// "who has COM7" — the Radar panel (its ADS-B and FormationFlight rows) and the main connection's port
// list (ConnectionControls: desktop toolbar, phone popout, link manager) all ask here.

import { defaultNetPort } from '$lib/stores/connection';
import type { RadarLinkFields, RadarLinkTransport, RadarSettings } from '$lib/stores/settings';

/** Which local radar source a link belongs to — picks its protocol's standard network port. */
export type RadarLinkKind = 'adsb' | 'formationFlight';

/** ADS-B receivers speak MAVLink, the FormationFlight module MSP. */
const LINK_PROTOCOL: Record<RadarLinkKind, string> = { adsb: 'mavlink', formationFlight: 'msp' };

/** Standard UDP port per source kind. Not the protocol's 14550: that is where the main link listens, and
 *  two listeners cannot share a port. 14551 / 14552 are the next two (also the port set of a Kite-Link module). */
const UDP_DEFAULT_PORT: Record<RadarLinkKind, number> = { adsb: 14551, formationFlight: 14552 };

/** Standard network port of a source kind on a transport, or null (serial). */
function standardNetPort(kind: RadarLinkKind, transport: RadarLinkTransport): number | null {
  if (transport === 'udp') return UDP_DEFAULT_PORT[kind];
  return defaultNetPort(LINK_PROTOCOL[kind], transport) ?? null;
}

/** The transport of a stored source; anything unknown or missing is serial (configs saved before TCP / UDP). */
function transportOf(src: Partial<RadarLinkFields>): RadarLinkTransport {
  return src.transport === 'tcp' || src.transport === 'udp' ? src.transport : 'serial';
}

/** A network port as the backend accepts it: an integer in 1..65535, otherwise 0 (= not started). */
function cleanNetPort(port: unknown): number {
  const p = typeof port === 'number' && Number.isFinite(port) ? Math.round(port) : 0;
  return p >= 1 && p <= 65535 ? p : 0;
}

/** The link as the Radar panel shows and edits it — and as it is pushed to the backend, so what is shown is
 *  what runs. Without serial ports (iOS) a serial source becomes TCP with an empty host (stays stopped until
 *  a host is entered). An IP link without a usable port gets its protocol's standard port. */
export function effectiveLink<T extends RadarLinkFields>(src: T, kind: RadarLinkKind, serialAvailable: boolean): T {
  let transport = transportOf(src);
  let host = src.host ?? '';
  if (transport === 'serial' && !serialAvailable) {
    transport = 'tcp';
    host = '';
  }
  const netPort = cleanNetPort(src.netPort) || (transport === 'serial' ? 0 : standardNetPort(kind, transport) ?? 0);
  return { ...src, transport, host, netPort };
}

/** Switch a source to another transport. The network port follows to the new transport's standard port
 *  unless the user set one of their own (anything but 0 or the old transport's standard port). */
export function withTransport<T extends RadarLinkFields>(src: T, kind: RadarLinkKind, next: RadarLinkTransport): T {
  const prev = transportOf(src);
  const port = cleanNetPort(src.netPort);
  const wasDefault = port === 0 || port === standardNetPort(kind, prev);
  const netPort = wasDefault && next !== 'serial' ? standardNetPort(kind, next) ?? port : port;
  return { ...src, transport: next, netPort };
}

/** One-line link label for a running source: the serial port, or `TCP host:port` / `UDP host:port`. */
export function linkSummary(src: RadarLinkFields): string {
  const t = transportOf(src);
  if (t === 'serial') return src.port;
  if (src.host) return `${t.toUpperCase()} ${src.host}:${src.netPort}`;
  return t === 'udp' && src.netPort ? `UDP :${src.netPort}` : t.toUpperCase();
}

// ── Serial-port claims ──────────────────────────────────────────────────────────────────────────────

/** Who holds a serial port: an ADS-B receiver (by row), the FormationFlight module, or the main link. */
export type PortOwner =
  | { kind: 'adsb'; index: number; name: string }
  | { kind: 'formationFlight' }
  | { kind: 'main' };

/** Serial port → the owners claiming it (normally one; a stale config may list a port twice). */
export type PortClaims = Map<string, PortOwner[]>;

/**
 * Serial ports claimed by the radar sources (and, if given, the main connection).
 * - `activeOnly: true` — what the main connection must avoid: only sources that run right now (radar master
 *   switch on, the system enabled and, for an ADS-B receiver, the receiver itself enabled).
 * - `activeOnly: false` — what the Radar panel must avoid: every configured source of an enabled system,
 *   running or not (an ADS-B receiver that is switched off still has its port reserved, as before).
 * Only sources whose transport is serial claim a port; an IP source's stale `port` field claims nothing.
 */
export function serialPortClaims(
  radar: RadarSettings,
  opts: { activeOnly: boolean; mainPort?: string | null },
): PortClaims {
  const claims: PortClaims = new Map();
  const add = (path: string, owner: PortOwner) => {
    if (!path) return;
    const list = claims.get(path);
    if (list) list.push(owner);
    else claims.set(path, [owner]);
  };
  if (opts.mainPort) add(opts.mainPort, { kind: 'main' });
  const systemsOn = !opts.activeOnly || radar.enabled;
  if (systemsOn && radar.adsb.enabled) {
    radar.adsb.local.forEach((l, index) => {
      if (opts.activeOnly && !l.enabled) return;
      if (transportOf(l) === 'serial') add(l.port, { kind: 'adsb', index, name: l.name });
    });
  }
  const ff = radar.formationFlight;
  if (systemsOn && ff.enabled && transportOf(ff) === 'serial') add(ff.port, { kind: 'formationFlight' });
  return claims;
}

/**
 * Who else listens on UDP port `port`, or null. A UDP link binds the local port with the number the user
 * entered; a second listener on the same number gets no data (the backend refuses to start it), so the
 * Radar panel warns before that happens. Counts every configured UDP source of an enabled system and the
 * main connection's UDP port (`mainUdpPort`: the port set for the main link while UDP is selected there,
 * connected or not — a radar source that binds it first would push the main link onto another port).
 */
export function udpPortOwner(
  radar: RadarSettings,
  self: PortOwner,
  port: number,
  opts: { mainUdpPort: number | null; serialAvailable: boolean },
): PortOwner | null {
  if (!port) return null;
  if (opts.mainUdpPort === port) return { kind: 'main' };
  const listensOn = (src: RadarLinkFields, kind: RadarLinkKind): boolean => {
    const link = effectiveLink(src, kind, opts.serialAvailable);
    return link.transport === 'udp' && link.netPort === port;
  };
  if (radar.adsb.enabled) {
    for (let index = 0; index < radar.adsb.local.length; index++) {
      const l = radar.adsb.local[index];
      const owner: PortOwner = { kind: 'adsb', index, name: l.name };
      if (!sameOwner(owner, self) && listensOn(l, 'adsb')) return owner;
    }
  }
  const ff = radar.formationFlight;
  if (ff.enabled && self.kind !== 'formationFlight' && listensOn(ff, 'formationFlight')) return { kind: 'formationFlight' };
  return null;
}

function sameOwner(a: PortOwner, b: PortOwner): boolean {
  if (a.kind === 'adsb' && b.kind === 'adsb') return a.index === b.index;
  return a.kind === b.kind;
}

/** The first owner of `path` other than `self`, or null when the port is free for `self`. */
export function claimOf(claims: PortClaims, path: string, self?: PortOwner): PortOwner | null {
  return claims.get(path)?.find((o) => !self || !sameOwner(o, self)) ?? null;
}

/** Display name of an owner (`tr` = the i18n formatter). */
export function ownerLabel(owner: PortOwner, tr: (key: string) => string): string {
  switch (owner.kind) {
    case 'adsb':
      return owner.name || '—';
    case 'formationFlight':
      return tr('radar.formationFlight');
    case 'main':
      return tr('radar.mainConnection');
  }
}
