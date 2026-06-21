# Stick / Gimbal Overlay — Plan

> 📦 ARCHIVED (2026-06-21) — Shipped (replay-only gimbal overlay; ROADMAP done). Kept for reference.

Two animated transmitter-gimbal indicators shown beside the replay player, à la Blackbox Explorer.
**Replay-only** (gimmick/analysis feature) — no live recording, no Rust changes. Agreed 2026-06-19.

## Scope & decisions (locked with the user)

- **Replay-only.** Live recording is **not** touched: the only live RC we already receive is MAVLink
  `RC_CHANNELS` at ~1 Hz — useless for a stick view and not worth sacrificing telemetry bandwidth to
  raise. The sticks read the **log-imported** RC columns, which are free and high-rate.
- **Two layers (INAV):** primary **theme-blue** dot = `rc_command` (rcCommand — always logged), plus a
  **secondary orange, dimmed** dot rendered behind it = `rc_data` (rcData = RAW RC straight from the TX,
  only when the log recorded it). This makes the FC overriding the stick in self-level / nav modes
  visible (rcCommand ≠ rcData).
- **ArduPilot `.bin`:** only `rc_data` (RCIN) is logged → it drives the **blue primary**, no secondary.
- **`.tlog` / live-recorded flights:** no RC columns → overlay simply hidden.
- **Mode 2 only** (95 % of pilots): left = throttle (Y) + yaw (X), right = pitch (Y) + roll (X).
  Extendable to modes 1/3/4 later if asked.
- **Look:** two transparent glassmorphism panels (same style as the LogPlayer bar, blur + theme border)
  to the **right** of the player control bar. Dots get a thin black ring for contrast.

## Data notes (why an adapter is needed)

- **Channel order differs:** INAV arrays (`rcCommand` *and* `rcData`) are `[Roll, Pitch, Yaw, Throttle]`;
  ArduPilot RCIN is **AETR** = `[Roll, Pitch, Throttle, Yaw]`. → `rc_data` order depends on `fc_variant`.
- **Normalization differs:** rcData / RCIN are µs (1000–2000, centre 1500); INAV `rcCommand` has
  roll/pitch/yaw already centred at 0 (±500) and throttle in µs.
- Discriminator: the mapping is **data-driven** (rcCommand present ⇒ INAV layering) except the raw-RC
  channel order, which uses `fc_variant.startsWith('ardu')`.

## Files (frontend only)

- `helpers/stickInput.ts` — pure adapter: parse `rc_command_json` / `rc_data_json` → normalized Mode-2
  `StickData { primary, secondary }` (−1…+1, +y up). Per-firmware order + scaling. Reusable/testable.
- `components/sticks/GimbalStick.svelte` — **reusable** single gimbal panel: glass panel + square field +
  crosshair + primary (blue) / optional secondary (orange, behind) dot + label. Pure props.
- `components/sticks/StickOverlay.svelte` — two `GimbalStick`s (Mode 2) + optional legend; positioned
  right of the centred 800px LogPlayer bar.
- `LogPlayer.svelte` — derives the current sample's `StickData` from `playbackTrack[playbackIndex]` and
  renders `<StickOverlay>` when present (gated on `showPlayer`).
- i18n: `player.stickThrottleYaw / stickPitchRoll / stickCommand / stickRaw` (en/de/fr).

## Reuse hook

`GimbalStick` is intentionally chrome-complete and store-free (values via props) so it can be reused for
a future feature (e.g. a live RC HUD or a dual-pilot / trainer view) without change.

## Possible follow-ups (not now)

- Stick modes 1/3/4 + channel-map (AETR/TAER) as settings.
- A RAW/Command toggle if showing both at once is too busy.
- Live HUD reuse (would need a cheap live RC source — currently none).
