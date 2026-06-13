# UAV Geozones / No-Fly Zones — explicit drone restriction maps

> **Status: TBD / deferred (2026-06-13).** Not on the roadmap. Direction note only — captured so a later
> effort starts from research, not zero. Gated on the global geozone-data landscape maturing (ED-318 rollout)
> and on resolving the licensing/commercial question per source.

## Why this is its own thing (not the airspace overlay)

We already render generic **ICAO airspaces** via OpenAIP ([Airspace Manager](../active/AIRSPACE_MANAGER.md),
ADR-038). Those are written for **manned aviation**. UAV operators need **explicit drone geozones** — the
zones a drone may **not** enter, or needs authorization for — as defined by the actual UAV rules (EU Reg.
2019/947 §15, German §21h LuftVO, FAA Part 107, …). They carry different semantics than an airspace:
zone **type** (prohibited / req-authorization / restricted / conditional), **reason**, **lower/upper** limits
with an **AGL/AMSL** reference, **validity time windows**, and a human-readable message.

**OpenAIP has no dedicated UAS-geozone layer** (airspaces + airports only), so we do not get NFZ from our
existing provider.

## The core constraint: worldwide → multi-provider is unavoidable

The app is distributed online and used worldwide, so we **cannot** rely on a single-country source. No free,
global, commercial-clean, **vector** NFZ API exists today. Every authority/provider works differently (format,
fields, licence), per country. The real engineering problem is taming that heterogeneity — not picking "the"
provider.

## Direction (fits the ADR-038 pluggable pattern)

- A separate **`GeozoneProvider` trait, one adapter per source** — the same shape as the `AeroProvider`.
- **Normalize everything to one internal model = EUROCAE ED-269 / ED-318** (the EU legal common-format;
  FAA and others map into it). This is **the key early decision**: it determines how painlessly new
  countries/providers plug in later. All map rendering, proximity alerts, and clearance checks see only this
  model; the "every provider differs" mess stays inside adapters.
- Richer model than our `Airspace`: type, reason, lower/upper + datum, validity, message.
- **Vector where available** (enables alerts/clearance); **raster overlay** (WMS) only as a fallback for
  sources that offer nothing else (display-only, no alerts).

## Data landscape (June 2026)

| Source | Coverage | Format | Licence / cost | Notes |
|---|---|---|---|---|
| **FAA UAS Facility Maps** (UDDS) | US only | **Vector** (ArcGIS REST / Open Data) | **Public domain** | The clean drop-in; first vector provider candidate |
| **DIPUL / DFS** | Germany | **WMS (raster)** | **Non-commercial only** + attribution ("Quelle Geodaten: DFS, BKG") | Authoritative DE; raster → no geometry/alerts; licence blocks our commercial-support model |
| **EASA IAM Hub** (ED-318) | EU (harmonized) | ED-318 vector | TBD | Goal = single EU feed, but **only test data (IE/ES/DK)** so far |
| National NAAs | per country | ED-269 / WMS / proprietary | varies | The fragmented reality; per-country adapters |
| Aggregators (GeoZones.eu, Altitude Angel, Aloft) | EU / multi | API | **Commercial / ToS-restricted** | Heikel for GPL + support model |

## Open questions

- Licence model: user-supplied keys per provider (like the OpenAIP/Cesium pattern) vs bundled sources — and
  how to honor non-commercial sources (e.g. DIPUL) when Kite ships under GPL + paid support.
- Raster (WMS overlay, no alerts) vs vector (alerts/clearance) — per-source capability tiers in the UI.
- Whether to reuse the Airspace Manager panel/subsystem or add a sibling "Geozones" layer set.
- Alert integration: feed geozones into the same conflict/proximity machinery as ADS-B (ADR-035)?

## Sources

- EASA — Where to fly / Geo Zones: https://www.easa.europa.eu/en/domains/drones-air-mobility/operating-drone/where-fly-easa-member-states-geo-zones
- EUROCAE ED-318: https://www.eurocae.net/ed-318-technical-specification-for-geographical-zones-and-u-space-data-provision-and-exchange/
- EUROCONTROL — UAS no-fly areas directory: https://www.eurocontrol.int/tool/uas-no-fly-areas-directory-information-resources
- DIPUL — Geographical zones + WMS: https://dipul.de/homepage/en/information/geographical-zones/ · https://dipul.bund.de/homepage/en/information/geographical-zones/web-map-service-wms/
- FAA UAS Facility Maps + Data Delivery System: https://www.faa.gov/uas/commercial_operators/uas_facility_maps · https://udds-faa.opendata.arcgis.com/
