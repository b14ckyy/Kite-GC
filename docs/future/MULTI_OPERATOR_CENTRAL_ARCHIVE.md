# Multi-Operator Central Flight Archive (exploratory)

> **STATUS: NOT PLANNED / EXPLORATORY** — captured for direction only. Gated on the app going
> public and generating revenue; revisit as a feature request after the standalone product is
> finished and proven. Nothing here is committed scope.

## The scenario

A company runs **several operators in different regions**, each using Kite GC standalone with its
**local SQLite DB**. The company wants a **central, company-wide flight archive**: every
operator's flights end up in one searchable place, without forcing each operator to carry the
whole (multi-GB) archive around.

## Key reframe (before anything else)

This is **not a "sync the SQLite database" feature — it is a small backend product.** Server +
storage + auth + API. Frame the effort accordingly.

Two assumptions to correct up front:

1. **No shared SQLite over SMB/VPN.** SQLite over network filesystems (SMB/NFS) has broken locking
   semantics → corruption, and no real multi-writer support. A central SQLite file shared by many
   clients is a non-starter.
2. **No two-way full replication.** Operators **push** their own data; they do **not** pull a full
   local replica of the archive. Browsing the archive is an **on-demand server query** (paginated).
   This directly removes the "central DB grows to many GB that everyone has to carry" problem.

**Recommended shape:** local SQLite stays as the **local store + outbox** → **HTTPS sync API** →
**central server DB (PostgreSQL) for metadata** + **object storage (S3/MinIO/Azure Blob) for the
large blobs**. One-way push + on-demand archive browsing. Multi-tenant auth.

## Identity & collision handling

- **Do not** use the local autoincrement DB id as global identity. Assign each syncable object a
  **global UUID (v7 / ULID, time-sortable)** at creation → no cross-operator collisions by design.
- **Content hash = dedup**, not identity. It detects "the same blackbox imported by two operators";
  identity stays the UUID.
- Server ingest is **idempotent** (upsert by UUID + hash) so a retried push never duplicates.

## Considerations checklist

### Offline-first & sync mechanics
- **Outbox pattern**: a sync-queue table with state (pending / uploading / synced / failed),
  retry count, last attempt. Local writes always succeed; sync is background reconciliation.
- **Metadata first, blobs later**: upload the small flight metadata immediately (the archive
  "sees" the flight), defer the heavy blackbox blob as a **chunked, resumable** upload that
  resumes after a dropped connection.
- **Bandwidth policy**: Wi-Fi-only, size caps, throttling, scheduling, metered-connection
  awareness; manual "sync now" + automatic background sync when connectivity allows.
- **Compression** (blackbox text → zstd/gzip) + **content-addressed storage** (store identical
  logs once).

### Data model additions (per syncable entity)
`global_id` (UUID), `operator_id`, `device_id`, `tenant/org_id`, `created_at` / `updated_at`
(UTC), `sync_state`, `content_hash`, `schema_version`.

### Referential integrity / the graph (the tricky part)
- A flight references the **mission library** and **battery packs** (+ `linked_flight_id`). These
  need global IDs too and must sync in a **consistent order** (mission/battery before the flight)
  or as a **bundled package**. The existing **`.kflight`** exchange format is a good basis for the
  wire payload.
- Decide whether missions/batteries are **shared across operators** (company mission library, same
  physical battery) — if so, define merge rules.

### Edits after sync ("two-way light")
- Editing notes / weather / craft name / platform type **after** a flight synced must push the
  update up (`updated_at`; last-writer-wins or field-level merge).
- **Deletes**: local delete ≠ archive delete. The archive is append-only/permanent →
  soft-delete / tombstone, never hard-delete server-side.
- **Local retention**: after a confirmed sync the operator may purge locally (server = source of
  truth).

### Security (much more than "encrypted connection")
- **Transport**: TLS; optional mTLS.
- **AuthN**: per operator/device (OAuth2/OIDC or device tokens) with rotation; client secret in
  the **OS keychain**, not in the settings/DB.
- **AuthZ + multi-tenancy**: company = tenant; operator writes only own/tenant data, admin reads
  all; hard tenant isolation.
- **At-rest encryption** (server DB + object storage), **audit log** (who uploaded/accessed what).
- **Tamper-evidence**: signed records / hash chain if logs are used for legal / insurance /
  incident-investigation purposes.

### Compliance (commercial drone ops, EU)
- **GDPR**: flight data includes pilot names + GPS tracks = personal data → data residency,
  retention periods, right to access/erasure.
- **EASA record-keeping** for commercial operations → possible immutability, defined retention,
  export for authorities.

### Schema / protocol evolution across the fleet
- Operators run **different app/schema versions**. The sync needs a **versioned wire-format
  negotiation** (forward/backward compatible), decoupled from the internal SQLite schema; server
  migrations independent of clients.

### Operations / infra
- Hosting (cloud vs. on-prem company server), **backups/DR**, monitoring, storage scaling
  (cold storage for old blobs), cost model, rate-limiting / abuse protection.
- **Telemetry rows**: many per flight → sync batched/compressed, or store as a compressed blob
  for archival.

### UX
- Per-flight sync status (local / syncing / synced / failed) + a global indicator; a
  "pending uploads" queue with manual retry; error surfacing; an **archive browser** (server-side
  search across all operators); data-usage settings.

## Build vs. buy

Offline-first sync engines exist and are relevant because the stack already uses SQLite:

- **Turso / libSQL** — SQLite-compatible, embedded replicas + central sync (edge ↔ server). Very
  close to the existing stack, but "only own data / partial" must be modelled on top.
- **PowerSync / ElectricSQL** — Postgres backend ↔ SQLite client with **partial replication**
  (sync rules). Fits offline-first + "don't pull everything".
- **CouchDB / PouchDB** — classic offline-first, but weak at large-blob handling.
- **Custom API** — maximum control (especially blob/bandwidth handling), maximum effort.

**Caveat for all off-the-shelf engines:** large blobs + "sync only a subset" are exactly their
weak corners. Realistically a **hybrid**: an engine/custom API for metadata sync + separate
object storage with resumable, content-addressed uploads for the blackbox blobs.

## Tentative recommended direction

Local SQLite as outbox → HTTPS sync API → PostgreSQL (metadata) + S3-compatible object storage
(blobs); global UUIDs; metadata-first + deferred resumable blob upload; one-way push + on-demand
archive browsing; multi-tenant auth.

## Open questions (decide when it becomes real)

- Hosting model: company-managed cloud vs. on-prem vs. a SaaS offering tied to the product?
- Are missions / batteries shared at the company level or strictly per operator?
- Do operators ever need to *read* other operators' flights in the field, or only admins via the
  archive browser?
- Regulatory bar: is this just an internal archive, or must it satisfy EASA/insurance record
  requirements (→ immutability + retention)?
- Build-vs-buy decision and the resulting dependency/cost footprint.

## When this graduates

Move a real plan into `docs/active/`, add ROADMAP entries, and write ADR(s) for the concrete
decisions (transport, identity scheme, storage split, auth model). Leave a pointer here.
