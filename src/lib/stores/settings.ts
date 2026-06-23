// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Persistent application settings
// Stores user preferences in localStorage, survives between sessions

import { writable } from 'svelte/store';
import type { RelayConfig } from '$lib/stores/relay';

export interface MapState {
  center: [number, number];
  zoom: number;
}

export interface PanelConfig {
  bottom: string[]; // widget IDs in display order
  right: string[];  // widget IDs in display order
  /** Remembers last panel assignment per widget so toggle off/on restores position */
  positions?: Record<string, 'bottom' | 'right'>;
}

export type SpeedUnit = 'kmh' | 'mph' | 'ms' | 'fts' | 'kt';
export type AltitudeUnit = 'm' | 'ft';
export type DistanceUnit = 'metric' | 'imperial';
export type VerticalSpeedUnit = 'ms' | 'fts';
export type TemperatureUnit = 'c' | 'f';
export type NightMode = 'off' | 'auto' | 'on';
/** GCS marker behaviour: hidden / placed once + draggable / live OS tracking. */
export type GcsMode = 'off' | 'manual' | 'continuous';

export interface InterfaceSettings {
  speedUnit: SpeedUnit;
  altitudeUnit: AltitudeUnit;
  distanceUnit: DistanceUnit;
  verticalSpeedUnit: VerticalSpeedUnit;
  temperatureUnit: TemperatureUnit;
}

/** One online ADS-B provider. `url` is a template with `{lat}`/`{lon}`/`{dist}` placeholders
 *  (`dist` filled in NM by the backend). */
export interface AdsbOnlineProvider {
  name: string;
  url: string;
  apiKey?: string;
  enabled: boolean;
}

/** A local hardware ADS-B receiver (MAVLink ADSB_VEHICLE). Phase 2: serial; TCP later. */
export interface AdsbLocalSource {
  name: string;
  transport: 'serial' | 'tcp';
  /** serial */
  port: string;
  baud: number;
  /** tcp (later) */
  host?: string;
  tcpPort?: number;
  enabled: boolean;
}

/** Fixed, always-present ADS-B providers (URL defined in code, not editable/deletable). Only their
 *  on/off state is persisted (in `radar.adsb.builtins`). */
export const BUILTIN_ADSB_PROVIDERS: { name: string; url: string }[] = [
  { name: 'adsb.lol', url: 'https://api.adsb.lol/v2/point/{lat}/{lon}/{dist}' },
  { name: 'adsb.fi', url: 'https://opendata.adsb.fi/api/v3/lat/{lat}/lon/{lon}/dist/{dist}' },
];

/** Radar (foreign-vehicle tracking) settings: master switch + per-system enables + per-system
 *  source config (ADS-B online from Phase 1; more added per phase). */
export interface RadarSettings {
  /** Master switch — off hides the whole Radar panel/feature. */
  enabled: boolean;
  adsb: {
    enabled: boolean;
    /** On/off state for the fixed built-in providers, keyed by name (URL lives in code). */
    builtins: Record<string, boolean>;
    /** User-editable custom providers (e.g. adsb.fi example). Merged with the built-ins by ICAO. */
    online: AdsbOnlineProvider[];
    /** Local hardware receivers (serial MAVLink; TCP later). */
    local: AdsbLocalSource[];
    /** Pull the ADS-B list from the connected UAV via MSP (INAV 8.0+). Bandwidth-heavy → opt-in. */
    mspFromFc: boolean;
    /** Query radius in km — dropdown 10/25/50/75/100, capped at 100. */
    radiusKm: number;
    /** Poll interval in seconds (provider limit ≈ 1 req/s, so ≥2 s). */
    pollSec: number;
  };
  formationFlight: RadarFormationFlightSettings;
  radio: { enabled: boolean };
  /** Map rendering of foreign contacts (2D + 3D). */
  map: RadarMapSettings;
  /** Conflict-alert stage switches (see RADAR_ALERTS.md). Numeric thresholds live in code for now. */
  alerts: RadarAlertSettings;
  /** Dev-only synthetic source (ignored by release backend). */
  sim: boolean;
}

/** FormationFlight (INAV-Radar / ESP32): one serial module Kite speaks MSP to as an emulated FC.
 *  See docs/active/RADAR_FORMATION_FLIGHT.md. */
export interface RadarFormationFlightSettings {
  enabled: boolean;
  /** Serial port the ESP32 module is on. */
  port: string;
  /** Baud (default 115200). */
  baud: number;
  /** Name we advertise via MSP_NAME (our node's broadcast name). Empty ⇒ a default is used. */
  nodeName: string;
}

/** Conflict-alert toggles. Only the two stage switches are user-facing for now; the numeric parameters
 *  stay in `ALERT_CONFIG` (controllers/radarAlerts.ts) until per-user tuning is added (RADAR_ALERTS §5). */
export interface RadarAlertSettings {
  /** Stage 1 — proximity warn-zone (caution). */
  stage1Enabled: boolean;
  /** Stage 2 — predicted closest-approach (warning). */
  stage2Enabled: boolean;
  /** Synthesised tone cue on alert. */
  soundEnabled: boolean;
  /** Spoken callout ("Traffic" / "Collision") on alert. */
  voiceEnabled: boolean;
}

/** Map-rendering controls for radar contacts (panel "Map" tab). See RADAR_TRACKING_PANEL_AND_MAP §4. */
export interface RadarMapSettings {
  /** Soft-dim radius (km): contacts beyond render dimmed + smaller, never hidden. */
  radiusKm: number;
  /** Absolute altitude ceiling (m): ADS-B contacts above are hidden from the map (always kept in the
   *  panel list). Overridden for any contact within +2000 m relative to the reference. */
  maxAltM: number;
  /** Show everything — disables the radius dim + the absolute altitude cutoff. */
  showAll: boolean;
  /** Per-system map visibility, independent of the data-enable in Settings. */
  visible: { adsb: boolean; formationFlight: boolean; radio: boolean };
}

/** Default radar settings — built-ins (adsb.lol/.fi) on; adsb.one as a custom (off — currently
 *  unreachable). */
export const DEFAULT_RADAR: RadarSettings = {
  enabled: true,
  adsb: {
    enabled: true,
    builtins: { 'adsb.lol': true, 'adsb.fi': true },
    online: [
      { name: 'adsb.one', url: 'https://api.adsb.one/v2/point/{lat}/{lon}/{dist}', enabled: false },
    ],
    local: [],
    mspFromFc: false,
    radiusKm: 25,
    pollSec: 5,
  },
  formationFlight: { enabled: false, port: '', baud: 115200, nodeName: '' },
  radio: { enabled: false },
  map: {
    radiusKm: 50,
    maxAltM: 10000,
    showAll: false,
    visible: { adsb: true, formationFlight: true, radio: true },
  },
  alerts: {
    stage1Enabled: true,
    stage2Enabled: true,
    soundEnabled: true,
    voiceEnabled: true,
  },
  sim: false,
};

/** Airspace Manager — aeronautical-data provider (one active at a time). See docs/active/AIRSPACE_MANAGER.md. */
export type AirspaceProvider = 'none' | 'openaip';
/** Per-layer 2D / 3D visibility for the four aero layers (panel-controlled, persisted). */
export interface AeroLayerVis { d2: boolean; d3: boolean }
export type AeroLayers = Record<'airspaces' | 'geozones' | 'fence' | 'obstacles' | 'airports' | 'rc', AeroLayerVis>;
/** Selectable render / list ranges (km) for the dense point layers. */
export const AERO_DISTANCE_OPTIONS = [1, 2, 5, 10, 15, 25] as const;

/** Airspace Manager global settings (Data tab) + the panel's persisted view state. */
export interface AirspaceSettings {
  /** Global feature toggle — enables the subsystem + shows the panel in the nav rail. */
  enabled: boolean;
  /** Active aeronautical-data provider. */
  provider: AirspaceProvider;
  /** Provider API key (user-supplied; persisted). */
  apiKey: string;
  /** Per-layer 2D/3D visibility (panel toggles). */
  layers: AeroLayers;
  /** Obstacle render (3D) + list range in km (horizontal, from the camera/reference). */
  obstacleDistanceKm: number;
  /** Airport + RC-airfield render + list range in km (shared). */
  airfieldDistanceKm: number;
  /** Panel collapsed to the compact (list-only) view. */
  compact: boolean;
}

export const DEFAULT_AIRSPACE: AirspaceSettings = {
  enabled: true,
  provider: 'none',
  apiKey: '',
  layers: {
    airspaces: { d2: false, d3: false },
    geozones: { d2: true, d3: true },
    fence: { d2: true, d3: true },
    obstacles: { d2: true, d3: true },
    airports: { d2: true, d3: true },
    rc: { d2: true, d3: true },
  },
  obstacleDistanceKm: 5,
  airfieldDistanceKm: 15,
  compact: false,
};

// ── RC Control (INAV RC over MSP — see docs/archive/MSP_RC_CONTROL.md) ──────────────────────────────────
// Only lightweight UI state lives in settings. The actual channel mappings live in shareable profile
// FILES under Documents/KiteGC/HID-Profiles (see stores/rcProfiles.ts) — not in localStorage.

export interface RcControlSettings {
  /** Master switch — shows the RC nav-rail tab. Off by default (opt-in feature). */
  enabled: boolean;
  /** UUID of the device last worked with (re-selected on next open). */
  selectedUuid: string | null;
  /** Name of the active profile (re-selected on next open), or null. */
  activeProfile: string | null;
  /** RAW_RC stream rate in Hz (10/15/20/25). Default 10 — conservative for slow OTA links. */
  rawRateHz: number;
  /** Offline platform selection for the config editor (see docs/active/MAVLINK_RC_CONTROL.md §2). When a
   *  FC is connected the detected platform wins and locks; this is the choice used while disconnected. */
  platform: RcPlatform;
}

/** Flight-stack the RC-control config targets — drives the channel split + the send adapter. */
export type RcPlatform = 'inav' | 'ardupilot' | 'px4';

export const DEFAULT_RC_CONTROL: RcControlSettings = {
  enabled: false,
  selectedUuid: null,
  activeProfile: null,
  rawRateHz: 10,
  platform: 'inav',
};

/** FC system-message (STATUSTEXT) toast verbosity: off, errors only, warnings+errors, or everything. */
export type SystemMessagesLevel = 'off' | 'error' | 'warning' | 'all';

/** Backend diagnostic file-log verbosity. "debug" also captures info-level connection milestones.
 *  Written to `<AppData>/kite-gc/kite-gc.log` (portable: `data/`). See src-tauri/src/logging. */
export type LogLevel = 'off' | 'error' | 'warning' | 'debug';

export interface AppSettings {
  lastPort: string;
  lastBaud: number;
  lastProtocol: string;
  // Full last-used connection path (restored on startup so nothing has to be re-entered)
  lastTransport: string;
  lastHost: string;
  lastTcpPort: number;
  lastBleDevice: string;
  map: MapState;
  mapProvider: string;
  mapCacheMaxMB: number;
  navPanelOpen: boolean;
  activeTab: string;
  // Telemetry poll rates
  attitudeRateHz: number;
  positionRateHz: number;
  airspeedEnabled: boolean;
  /** Show the heading / course-over-ground / predicted-turn direction lines at the aircraft on the 2D map. */
  directionLines: boolean;
  /** Show INAV safehome markers + radius rings (+ autoland approaches) on the map. Default on; toggled
   *  in the Safe Home Manager (≥7.1). See docs/active/AUTOLAND_SAFEHOME.md. */
  showSafehomes: boolean;
  // MAVLink only: stream everything the FC sends (per its SRn_* params) instead of requesting our
  // reduced rate set — ignores the two rate knobs. For high-bandwidth links + full .tlog capture.
  mavlinkFullTelemetry: boolean;
  // Flight logging
  flightLoggingEnabled: boolean;
  flightRecordingEnabled: boolean;
  flightLogDbPath: string;
  flightLogRawPath: string;
  flightLogRawEnabled: boolean;
  flightLogRawAlways: boolean;
  // Mission Control
  defaultWpAltitudeM: number;
  defaultPhTimeSec: number;
  lastAutopilotSystem: string;
  /** Last ArduPilot vehicle class chosen for offline mission planning (VehicleClass string). */
  lastArduVehicleClass: string;
  // Alerts
  warnAltitudeM: number;
  /** Which FC system messages (MAVLink STATUSTEXT) surface as on-screen toasts. */
  systemMessages: SystemMessagesLevel;
  /** Backend diagnostic file-log verbosity (applied to the Rust logger on startup + on change). */
  logLevel: LogLevel;
  // Global UI options (display-only conversions)
  interface: InterfaceSettings;
  // Widget panel layout
  panels: PanelConfig;
  // Locale / language
  locale: string;
  /** Global UI scale factor (1 = 100%, up to 2 = 200%); zooms the chrome, not the map. */
  uiScale: number;
  // 3D Map
  cesiumIonToken: string;
  /** Show the vertical altitude curtain (wall down to ground) under the 3D track. */
  altitudeCurtain3D: boolean;
  /** Light the 3D globe with the real sun position (day/night terminator + shading). */
  realLighting3D: boolean;
  /** During replay, drive the 3D sun clock from the log's recorded timestamp (not wall-clock now). */
  logReplayTime: boolean;
  /** Dim the 2D Leaflet imagery for night: off / auto (sun below horizon) / on. */
  nightMode2D: NightMode;
  /** GCS marker mode: off / manual (drag) / continuous (live OS location). */
  gcsMode: GcsMode;
  /** Last known physical user location (for Night-Mode auto sunset timing); persisted across sessions. */
  userLocation: { lat: number; lon: number } | null;
  /** Radar (foreign-vehicle tracking) subsystem settings. */
  radar: RadarSettings;
  /** Airspace Manager (aeronautical data) global settings. */
  airspace: AirspaceSettings;
  /** Telemetry Relay (forwarding/conversion) configs — persisted, auto-connected on primary connect. */
  relays: RelayConfig[];
  /** RC Control (INAV RC over MSP) — HID device + channel mappings. */
  rcControl: RcControlSettings;
}

const STORAGE_KEY = 'kite-gc-settings';

const defaults: AppSettings = {
  lastPort: '',
  lastBaud: 115200,
  lastProtocol: 'msp',
  lastTransport: 'serial',
  lastHost: '192.168.1.1',
  lastTcpPort: 5761,
  lastBleDevice: '',
  map: {
    center: [51.505, -0.09],
    zoom: 13,
  },
  mapProvider: 'esri-hybrid',
  mapCacheMaxMB: 200,
  navPanelOpen: true,
  activeTab: 'uav-info',
  attitudeRateHz: 5,
  positionRateHz: 2,
  airspeedEnabled: false,
  directionLines: true,
  showSafehomes: true,
  mavlinkFullTelemetry: false,
  flightLoggingEnabled: true,
  flightRecordingEnabled: true,
  flightLogDbPath: '',
  flightLogRawPath: '',
  flightLogRawEnabled: false,
  flightLogRawAlways: false,
  defaultWpAltitudeM: 50,
  defaultPhTimeSec: 30,
  lastAutopilotSystem: 'inav',
  lastArduVehicleClass: 'plane',
  warnAltitudeM: 120,
  systemMessages: 'all',
  logLevel: 'warning',
  interface: {
    speedUnit: 'kmh',
    altitudeUnit: 'm',
    distanceUnit: 'metric',
    verticalSpeedUnit: 'ms',
    temperatureUnit: 'c',
  },
  panels: {
    bottom: ['home', 'speed', 'ahi', 'altitude', 'gps', 'compass'],
    right: ['flightMode', 'battery'],
  },
  locale: 'en',
  uiScale: 1,
  cesiumIonToken: '',
  altitudeCurtain3D: true,
  realLighting3D: true,
  logReplayTime: true,
  nightMode2D: 'auto',
  gcsMode: 'continuous',
  userLocation: null,
  radar: DEFAULT_RADAR,
  airspace: DEFAULT_AIRSPACE,
  relays: [],
  rcControl: DEFAULT_RC_CONTROL,
};

function load(): AppSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<AppSettings>;
      return {
        ...defaults,
        ...parsed,
        interface: {
          ...defaults.interface,
          ...(parsed.interface ?? {}),
        },
        radar: (() => {
          const dr = defaults.radar;
          const pr = (parsed.radar ?? {}) as Partial<RadarSettings>;
          const pa = (pr.adsb ?? {}) as Partial<RadarSettings['adsb']>;
          const builtinNames = new Set(BUILTIN_ADSB_PROVIDERS.map((p) => p.name));
          return {
            ...dr,
            ...pr,
            adsb: {
              ...dr.adsb,
              ...pa,
              builtins: { ...dr.adsb.builtins, ...(pa.builtins ?? {}) },
              // Strip any built-in-named entries from the custom list (migration from the old flat
              // `online` array, where adsb.lol/.one lived as custom rows).
              online: (pa.online ?? dr.adsb.online).filter((p) => !builtinNames.has(p.name)),
              local: pa.local ?? dr.adsb.local,
            },
            formationFlight: { ...dr.formationFlight, ...(pr.formationFlight ?? {}) },
            radio: { ...dr.radio, ...(pr.radio ?? {}) },
            map: {
              ...dr.map,
              ...(pr.map ?? {}),
              visible: { ...dr.map.visible, ...(pr.map?.visible ?? {}) },
            },
            alerts: { ...dr.alerts, ...(pr.alerts ?? {}) },
          };
        })(),
        airspace: (() => {
          const da = defaults.airspace;
          const pa = (parsed.airspace ?? {}) as Partial<AirspaceSettings>;
          const pl = (pa.layers ?? {}) as Partial<AeroLayers>;
          return {
            ...da,
            ...pa,
            layers: {
              airspaces: { ...da.layers.airspaces, ...(pl.airspaces ?? {}) },
              geozones: { ...da.layers.geozones, ...(pl.geozones ?? {}) },
              fence: { ...da.layers.fence, ...(pl.fence ?? {}) },
              obstacles: { ...da.layers.obstacles, ...(pl.obstacles ?? {}) },
              airports: { ...da.layers.airports, ...(pl.airports ?? {}) },
              rc: { ...da.layers.rc, ...(pl.rc ?? {}) },
            },
          };
        })(),
        rcControl: {
          ...DEFAULT_RC_CONTROL,
          ...(parsed.rcControl ?? {}),
        },
      };
    }
  } catch {
    // Ignore parse errors, use defaults
  }
  return { ...defaults };
}

function createSettingsStore() {
  const initial = load();
  const { subscribe, set, update } = writable<AppSettings>(initial);

  return {
    subscribe,
    set(value: AppSettings) {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
      set(value);
    },
    update(updater: (s: AppSettings) => AppSettings) {
      update((current) => {
        const next = updater(current);
        localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
        return next;
      });
    },
    /** Update a single field without replacing the whole object */
    patch(partial: Partial<AppSettings>) {
      update((current) => {
        const next = { ...current, ...partial };
        localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
        return next;
      });
    },
  };
}

export const settings = createSettingsStore();
