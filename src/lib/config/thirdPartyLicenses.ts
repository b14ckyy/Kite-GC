// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Notable bundled components + data sources for the About dialog (attribution). Curated, not exhaustive:
// GPL-3.0 doesn't require a dependency list, and permissive (MIT/BSD/Apache) notices live with the
// distribution / repo. The granular transitive crates carry compatible permissive licences.

export interface LicenseGroup {
  heading: string;
  items: { name: string; license: string }[];
}

export const THIRD_PARTY_LICENSES: LicenseGroup[] = [
  {
    heading: 'Components',
    items: [
      { name: 'Tauri (@tauri-apps)', license: 'MIT / Apache-2.0' },
      { name: 'Svelte / SvelteKit', license: 'MIT' },
      { name: 'CesiumJS', license: 'Apache-2.0' },
      { name: 'Leaflet', license: 'BSD-2-Clause' },
      { name: 'serialport-rs', license: 'MPL-2.0' },
      { name: 'Other Rust crates (serde, tokio, reqwest, rusqlite, …)', license: 'MIT / Apache-2.0' },
    ],
  },
  {
    heading: 'Data sources',
    items: [
      { name: 'Map tiles — © OpenStreetMap contributors & tile providers', license: 'ODbL / provider terms' },
      { name: 'Aeronautical data — © OpenAIP contributors', license: 'OpenAIP terms' },
      { name: 'Terrain — Copernicus GLO-30 DEM / Cesium World Terrain', license: 'Copernicus / Cesium terms' },
    ],
  },
];
