// Radar — data sources. One module per source family (added incrementally, see the phasing in
// docs/active/RADAR_TRACKING_CORE.md §9). Phase 0 ships only the dev-only `sim` source.

pub mod adsb_mavlink;
pub mod adsb_online;
pub mod sim;
