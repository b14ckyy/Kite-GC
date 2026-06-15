# External Control API + AI / Intent-Driven Mission Planning (exploratory)

> **STATUS: NOT PLANNED / GATED** — captured for direction only. Not committed scope, not the ROADMAP,
> not an ADR. Not for the initial release. Gated on a real demand signal (an external integrator, an
> AI-planning use-case that sticks) and on the safety work being worth it. Trigger that prompted this
> note: INAV outreach RFC for **URML** (a declarative robot-intent / mission-validation language) —
> [iNavFlight/inav#11651](https://github.com/iNavFlight/inav/issues/11651),
> [RFC-0577](https://github.com/URML-MARS/URML/blob/main/docs/rfcs/0577-inav-outreach.md). Noted 2026-06-15.

## The idea

Let **external tools drive Kite** — read app state (mission, track, telemetry, alerts, airspace) and
perform actions (set/edit waypoints, load/upload a mission, run app commands). The motivating case is
**AI / LLM-assisted mission planning**: an external planner proposes a mission, Kite validates it, then
hands it to the FC.

The single most important design rule, and the one the URML framing gets right:

> **The LLM is never in the control loop.** It *authors* intent. A formal, deterministic layer
> *validates* it against real constraints. The FC *executes*. Author → validate → fly.

This keeps a non-deterministic component (the LLM) strictly upstream of a deterministic safety gate.

## Why Kite is the right place for the "validate" step

INAV/ArduPilot enforce geometric bounds (geofence, altitude, failsafe). A GCS can do far more, because
**Kite already aggregates exactly the data a real safety envelope needs**:

- **Terrain** — terrain profile / clearance (`terrain_elevation`, Terrain Analysis): "never below X m AGL".
- **Airspace** — OpenAIP overlay (Airspace Manager): "no waypoint inside controlled airspace / near obstacles".
- **Traffic** — ADS-B / Radar + CPA conflict alerts: live and predicted conflicts.
- **Energy / range** — battery model, home distance.

So an external (LLM) planner that validates against **Kite** gets *real* terrain/airspace/traffic
checking, not just "lat/lon inside a box". That is the genuine differentiator — neither the firmware nor
an intent language alone can do it. **Kite is the layer that fills the envelope with real data.**

## How it maps onto the current architecture

What today's stack already gives us:

- **Unified backend event stream** — telemetry/home/nav-status/STATUSTEXT are emitted with the **same
  event names regardless of protocol** (MSP/MAVLink). That is effectively an internal pub/sub to expose.
- **Command pattern** — Tauri commands are typed `Result<T, String>` handlers; an external API is "the
  same handlers, a different entry point".
- **Scheduler owns the serial link exclusively** (one writer, strict request→response). For robustness
  this is a gift: external actions must **enqueue into the existing scheduler queue**, never touch serial
  directly — so there is never a second writer to the FC.

The two real challenges (unchanged from the general external-API question):

1. **State is split (Rust ↔ Svelte).** Backend: telemetry, FC state, serial. Frontend stores: mission,
   alerts/radar, airspace, and a lot of UI-orchestrated logic (mission editing lives in stores; upload
   is a separate command). An external call like "set a waypoint" touches logic that is currently in the
   frontend. Options: (a) **command-bus round-trip** — the Rust API forwards the action to the webview
   via an event with a correlation id, the frontend runs it through the **existing controllers** and
   replies (max reuse, needs the window running — fine for a desktop GCS); (b) move shared state into
   Rust (cleaner, bigger rewrite); (c) **hybrid** — reads + FC actions straight from Rust, frontend-owned
   actions via (a). **Hybrid is the recommendation.**
2. **Safety + serialization is the actual work**, not the transport.

## Proposed shape (when/if it happens)

A local server in the Rust backend (e.g. `axum` + WebSocket), bound to `127.0.0.1`, in three layers:

1. **Read / stream** — re-broadcast the existing event stream (telemetry, alerts, …) + snapshot reads
   (`get_mission`, `get_track`, `get_alerts`).
2. **Actions** — a **declarative action registry** (name → JSON schema → required scope → handler).
   Handlers run in Rust (FC/telemetry) or dispatch to the frontend via the command bus (mission/alerts).
   Versioned (`/v1`) with a capability/schema endpoint so clients self-discover.
3. **Safety gate** (the keystone) — auth, scope check, **intent + envelope validation**, queue
   enqueue, audit log. Central, not per-handler.

### Intent + envelope validation — the durable, reusable core

Independent of any external language, define **one internal intent model** ("goal + constraints") and an
**envelope validator** that runs every proposed mission through terrain / airspace / traffic / geofence /
range checks and returns **accepted, or rejected with reasons**. This is needed for *any* external
control (LLM or not). Keep it **schema-agnostic**: URML, MCP, or our own JSON are just **adapters** onto
the internal model — so "URML support" later is a mapper, not a rewrite.

### For AI agents specifically: MCP

The "external tool controls the app" shape is essentially **MCP (Model Context Protocol)**: *resources*
= stores (mission/track/alerts), *tools* = actions (set waypoint, load mission, validate intent). For the
LLM-planning use-case, target **MCP** rather than inventing a bespoke API — standardized, versioned, and
purpose-built for exactly this.

## Robustness checklist

- **Single writer to the FC** — enqueue into the scheduler queue; never a second serial writer.
- **Scopes / authz** — read-only vs. "control"; dangerous actions (arm, mode change, upload) behind an
  explicit per-session opt-in + token bound to localhost.
- **Validation & bounds** — schema-check every action; enforce terrain/airspace/alt/geofence
  server-side; reject with reasons.
- **Concurrency** — correlation ids, timeouts, backpressure, rate limiting, idempotency.
- **Versioned schema** + capability discovery so external tools don't break.
- **Audit log** of every external action.

## Assessment of URML specifically

- Right **pattern** (intent + envelope + handoff, LLM out of the loop). Worth designing toward.
- Wrong **maturity to adopt now**: RFC/outreach stage, no spec, AI-assisted single-author project,
  Apache-2.0. Betting on the *language* today would be premature.
- **Stance**: friendly "interesting, watching it" on the INAV issue; the real value is GCS-side; stay
  schema-agnostic so we can add a URML adapter cheaply *if* it gains traction.

## Phased effort (rough, not scheduled)

- **P1 — read-only API/stream** (telemetry/track/mission/alerts): low–medium (event stream exists; mostly
  server + auth + pulling mission/alerts snapshots from the frontend).
- **P2 — actions** (set waypoints, load/upload mission, app commands): medium (action registry +
  command-bus round-trip + queue enqueue).
- **P3 — hardening + intent/envelope validator + MCP wrapper**: the largest piece — but it is safety, not
  mechanics, and the envelope validator is the reusable keystone.

## Open questions

- Where does the boundary sit — does the envelope validator live in Rust (needs terrain/airspace data
  access from there) or in the frontend (where that data currently lives)? Likely pushes toward moving
  some envelope data/logic toward Rust, or a frontend-side validator reached via the command bus.
- How much of "control" do we ever expose? Mission authoring is reasonable; live in-flight commanding is
  a much higher bar.
- Human-in-the-loop confirmation for accepted-but-AI-authored missions before upload?
