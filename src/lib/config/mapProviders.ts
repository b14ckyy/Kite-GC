// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Map tile provider definitions for Leaflet
// All providers listed here are free and require no API key.

export interface MapProvider {
  id: string;
  label: string;
  url: string;
  attribution: string;
  maxZoom: number;
  /** Highest zoom the server has real tiles for, when lower than maxZoom — the 2D map enlarges
   *  those tiles above it instead of requesting levels that do not exist. */
  maxNativeZoom?: number;
  /** Max zoom for CesiumJS 3D view — lower than maxZoom for providers with sparse high-zoom coverage */
  cesiumMaxZoom?: number;
  /** Optional overlay layer URLs rendered on top of the base layer (e.g. labels on satellite) */
  overlays?: { url: string; attribution: string; maxZoom: number; maxNativeZoom?: number; cesiumMaxZoom?: number }[];
  /** True when the base imagery is dark (satellite, dark theme) — used for UI contrast */
  dark?: boolean;
  /** Run over-zoom placeholder detection on this base layer (ESRI satellite
   *  returns a fixed blank tile instead of a 404 above the available zoom). */
  detectPlaceholders?: boolean;
}

export const MAP_PROVIDERS: MapProvider[] = [
  {
    id: "osm",
    label: "OpenStreetMap",
    url: "https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png",
    attribution:
      '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    maxZoom: 19,
  },
  {
    id: "esri-street",
    label: "ESRI World Street Map",
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Street_Map/MapServer/tile/{z}/{y}/{x}",
    attribution:
      '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Sources: Esri, DeLorme, NAVTEQ',
    maxZoom: 20,
    cesiumMaxZoom: 17,
  },
  {
    id: "esri-satellite",
    label: "ESRI World Imagery",
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
    attribution:
      '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Sources: Esri, Maxar, Earthstar Geographics',
    maxZoom: 20,
    // Raised from 17 → 20 now that placeholder detection falls back per-region.
    cesiumMaxZoom: 20,
    dark: true,
    detectPlaceholders: true,
  },
  {
    id: "esri-hybrid",
    label: "ESRI Hybrid",
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
    attribution:
      '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Sources: Esri, Maxar, Earthstar Geographics',
    maxZoom: 20,
    // Raised from 17 → 20 now that placeholder detection falls back per-region.
    cesiumMaxZoom: 20,
    dark: true,
    detectPlaceholders: true,
    overlays: [
      {
        url: "https://server.arcgisonline.com/ArcGIS/rest/services/Reference/World_Boundaries_and_Places/MapServer/tile/{z}/{y}/{x}",
        attribution: "",
        maxZoom: 20,
        cesiumMaxZoom: 17,
      },
      {
        url: "https://server.arcgisonline.com/ArcGIS/rest/services/Reference/World_Transportation/MapServer/tile/{z}/{y}/{x}",
        attribution: "",
        maxZoom: 20,
        cesiumMaxZoom: 17,
      },
    ],
  },
  {
    id: "opentopomap",
    label: "OpenTopoMap",
    url: "https://{s}.tile.opentopomap.org/{z}/{x}/{y}.png",
    attribution:
      '&copy; <a href="https://opentopomap.org">OpenTopoMap</a> (<a href="https://creativecommons.org/licenses/by-sa/3.0/">CC-BY-SA</a>)',
    maxZoom: 17,
  },
  {
    // Replaces "CartoDB Dark Matter" (id cartodb-dark): CARTO's basemap servers answer every tile
    // with an "API KEY REQUIRED" image since November 2026.
    id: "esri-dark",
    label: "ESRI Dark Gray",
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}",
    attribution:
      '&copy; <a href="https://www.esri.com/">Esri</a> &mdash; Sources: Esri, HERE, Garmin, &copy; OpenStreetMap contributors',
    // Real tiles end at z16; above that the server sends a "Map data not yet available" image.
    maxZoom: 20,
    maxNativeZoom: 16,
    cesiumMaxZoom: 16,
    dark: true,
    overlays: [
      {
        url: "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Reference/MapServer/tile/{z}/{y}/{x}",
        attribution: "",
        maxZoom: 20,
        maxNativeZoom: 16,
        cesiumMaxZoom: 16,
      },
    ],
  },
];

/** Provider ids of earlier versions that no longer exist → their replacement. */
const REPLACED_PROVIDER_IDS: Record<string, string> = {
  "cartodb-dark": "esri-dark",
};

/** The id to use for a stored provider id (maps removed providers to their replacement). */
export function currentProviderId(id: string): string {
  return Object.hasOwn(REPLACED_PROVIDER_IDS, id) ? REPLACED_PROVIDER_IDS[id] : id;
}

export const DEFAULT_PROVIDER_ID = "osm";

export function getProviderById(id: string): MapProvider {
  const current = currentProviderId(id);
  return MAP_PROVIDERS.find((p) => p.id === current) ?? MAP_PROVIDERS[0];
}
