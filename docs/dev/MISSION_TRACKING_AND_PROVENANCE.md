# Mission Tracking & Provenance — flag model + active-WP highlight gating

**Status:** Spec frozen (2026-06-02). Active-WP **highlight** rendering is built; the **flag
model + gating** below is the next implementation step. Protocol-agnostic (INAV first,
ArduPilot layer follows).

The active-waypoint highlight (a pulsing green glow on the FC's current target WP) is only
meaningful when the mission shown on the map is *actually the one being flown/recorded*. This
doc defines a small **3-flag provenance model** that decides, simply and unambiguously, when the
highlight (and live/replay mission tracking) is trustworthy.

---

## The 3 flags (per mission slot)

Each mission **slot** (INAV multi-mission has several) carries up to three independent flags:

| Flag | Meaning |
|------|---------|
| **FC** | In sync with the connected flight controller (uploaded to **or** downloaded from it). |
| **FILE** | Loaded from / saved to a `.mission` (INAV) or `.waypoints` (ArduPilot) file. |
| **DB** | Loaded from / integrated into a log in the database (the flown mission). |

A **fresh** mission (just created/edited in the planner) has **no flags**.

## Flag transitions

| Event | Effect on flags |
|---|---|
| Create / add WP to an empty mission | fresh (no flags) |
| **Any WP edit** (add / move / delete / param / reorder) | **clear ALL flags of that slot** (until re-synced) |
| Undo back to a synced state | flags restored (snapshot history already covers this) |
| Save to file / load from file | set **FILE** |
| FC **upload** or **download** | set **FC** on exactly the synced slot(s); **clear FC on all other slots** |
| Integrate mission ↔ log (either direction, also manual) | set **DB** (and keep/set **FILE** if it came from a file) |
| **Disconnect** (manual) | clear **FC** only — **FILE/DB stay** (they're still valid); reconnect re-syncs |
| Clear mission | no flags (existing confirm dialog applies) |

**Multi-mission rule (INAV):** INAV 4.0+ stores several missions together (≤ 9 segments / 120 WP)
and can upload **all** or just the **active** one. The **FC flag is per slot** and marks exactly
the slot(s) currently on the FC: "upload all" → every slot gets FC; "upload active" / single →
only that slot gets FC and the others lose it (the FC no longer holds them). No ambiguity.

---

## Highlight / tracking gates

Render the active-WP highlight only when the loaded mission is trusted for the active context.
(`inWpMode` = FC reports NAV_WP flight mode — already implemented.)

- **Replay:** highlight when the slot has the **DB** flag. Otherwise a one-time popup
  **"Track loaded mission for replay?"** — asked **once** per replay session (on log load, if a
  mission is already on the map) and **once** per loaded mission file; not re-asked per frame.
- **Live:** highlight when the slot has the **FC** flag **and** the UAV is **Armed** (+ `inWpMode`).
  Otherwise a one-time popup **"Track loaded mission for flight?"** — asked **once at arm**.
- **Connect popup:** on connecting to a UAV, offer **Upload** / **Download** (only if the FC has a
  mission) / **Nothing**. Downloading replaces the map mission (with a replace-confirm if the map
  holds unsaved work).

The popup answer holds for the running session (replay session / current flight) until the
mission is changed or edited.

---

## Planned UI surface

- **Flag labels in the Mission panel** — show the active slot's flags (FC / FILE / DB) as small
  labels in the **bottom-right**, after the existing **"Modified"** label. _(reminder)_
- **Active WP in the Flight Mode widget** — when the FC is in **MISSION/WP mode**, show the
  current target WP number in the Flight Mode widget — **always**, independent of whether the
  mission is being tracked/highlighted on the map. _(reminder)_

---

## Dependencies & caveats (keep in mind)

- **DB flag needs the "mission in the log" DB schema** — storing the full mission (XML /
  `.waypoints`) with a recorded flight. Prepared now (flag exists), **wired later** (see the
  Mission-provenance workstream in `ROADMAP.md`). Until then the DB flag only comes via the manual
  integrate flow; the **FC/live path works immediately**.
- **"Upload all" + the combined separator format** — verify whether the GCS already uploads the
  full multi-mission blob or only the active mission; "upload all" may need building. Separate
  small item.
- **Live multi-mission in-flight switch** — the FC can switch the active mission mid-flight; the
  highlight assumes the displayed slot is the running one. Fully correct tracking needs the FC's
  **active-mission index** (if exposed via MSP) — a refinement, not a blocker.
- **Edit while armed** — clears the slot's flags → tracking stops mid-flight (pragmatic). The user
  can Undo back to the synced state, or re-upload the edited mission in flight (allowed outside WP
  mode) to restore FC sync.

---

## Current state (already implemented, gated only on `inWpMode` for now)

- `MSP_NAV_STATUS` (121) polled live → `telemetry-nav-status` event → `telemetry.activeWpNumber`.
- Replay: `active_wp_number` parsed from blackbox / ArduPilot logs → adapter → same field.
- `stores/navStatus.ts` (`activeWpNumber`), set in `+page` from the unified `telem` **only when in
  NAV_WP mode**.
- `InavMissionLayer` highlights the WP whose `number === activeWpNumber` with a pulsing green
  brightness+glow on the icon itself (0.5 Hz). FBH house included.

**Next:** add the flag model + the trust gates above, then surface the planned UI.
