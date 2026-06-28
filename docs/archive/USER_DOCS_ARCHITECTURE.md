# User Documentation — Architecture & Hosting Plan

> ARCHIVED (2026-06-28) — realised: the MkDocs Material site shipped to master (2026-06-27). Kept for the
> hosting/layout rationale and the deferred "For developers" section.
>
> STATUS: SHIPPED (2026-06-27). Decides **how user docs are presented, authored, and hosted** before
> any user-facing content is written, so the source layout is ingested by the site generator from day
> one. Dev-facing planning doc (lives in `docs/active/`); the published site is user-facing only.

## Goal
A public, **searchable documentation website** for end users (and testers), built from plain Markdown
files in the repo — the MWPTools-style "drop `.md` files, get a searchable site" experience. Authors
keep writing Markdown (what we already do); a generator turns `docs/user/` into a static site hosted
on **GitHub Pages**.

## Decision 1 — Site generator
**Recommendation: MkDocs + [Material for MkDocs].** It is the standard tool for "render a folder of
Markdown as a searchable site," and it fits our constraints best:
- Authors write **plain Markdown** — no React/MDX/framework to learn.
- **Built-in offline search** (client-side, no server) — the core requirement.
- **Material theme**: dark palette (matches our INAV-dark look), admonitions (tips/warnings), code
  highlighting, responsive nav, "edit this page" links.
- **One-file GitHub Pages deploy** via a GitHub Action.
- **Full local preview** with `mkdocs serve` (live-reload at `localhost:8000`) — we can build and test
  the *entire* site locally, which sidesteps the private-repo Pages limitation (see Decision 3).
- Optional later: versioned docs via `mike`, multi-language via the i18n plugin.

### Alternatives considered
| Tool | Pro | Con | Verdict |
|---|---|---|---|
| **MkDocs Material** | plain MD, great search, trivial Pages deploy, local preview | Python toolchain | **Recommended** |
| Astro **Starlight** | very fast, first-class i18n, Pagefind search | Node build, more config, MDX-ish | Alt if we want strong DE/FR docs early |
| **Docusaurus** | versioning + i18n + blog | React/MDX, heavyweight, overkill | Too much |
| **Docsify** | zero build (renders MD at runtime) | weaker search/SEO, no prerender | Only if we want no CI at all |

## Decision 2 — Source layout (authored to be ingested directly)
The site's source root (`docs_dir`) is **`docs/user/`**. Dev docs (`docs/`, `docs/active/`,
`docs/archive/`, `docs/future/`, `docs/reference/`) are **NOT published** — they stay internal. This
keeps one repo, clean separation, and lets the generator point at user content only.

```
docs/user/
  index.md                     # Landing: what Kite GC is, supported FCs (INAV / ArduPilot / PX4)
  getting-started/
    installation.md            # Download/install per OS; first run
    first-connection.md        # Pick transport, connect, what you should see
    quick-tour.md              # The main UI at a glance
  guides/
    connecting.md              # Serial/USB, Bluetooth-SPP (+ the error-121 tip), TCP/UDP, BLE, telemetry/relay modes
    telemetry-and-display.md   # HUD, compass/wind, units, widgets
    missions.md                # Plan/upload/download, multi-autopilot
    logbook.md                 # Recording, import (blackbox/tlog/rawmsp), replay
    vehicles.md                # Vehicle DB (craft-name link, FC write, stats baseline)
    batteries.md               # Battery DB (serials, lifetime, .kbatt)
    video.md                   # RTSP/go2rtc, surfaces, map↔video swap
    safety.md                  # Geofence / geozones / safehome / autoland
    radar-and-adsb.md          # Foreign-vehicle tracking + alerts
    map-3d.md                  # Cesium 3D, terrain, FPV cam
    relay-and-forwarding.md    # Telemetry relay/conversion
    rc-control.md              # GCS joystick/HID steering
  reference/
    settings.md                # Settings panel reference
    keyboard-shortcuts.md
    file-formats.md            # .kflight / .kbatt / .kvehicle / .rawmsp / mission formats
  troubleshooting/
    connection.md              # BT-SPP error 121, port busy, baud, BLE pairing
    video.md                   # ffmpeg/go2rtc, latency, codecs
    faq.md
  assets/                      # screenshots + images (referenced relatively)
```
Authoring conventions (so files render correctly on the site):
- **Relative Markdown links** between pages (`../guides/missions.md`) — MkDocs rewrites them.
- Images in `docs/user/assets/`, referenced relatively; keep them reasonably sized.
- Use Material **admonitions** for tips/warnings (`!!! tip`, `!!! warning`).
- One `#` H1 per page (the page title); nav is defined in `mkdocs.yml`.
- English is the primary language (matches the rest of the doc base); DE/FR can come later via the
  i18n plugin if wanted (see Open Questions).

## Decision 3 — Hosting (GitHub Pages) & the private-repo constraint
**Host on GitHub Pages, deployed by a GitHub Action** (`mkdocs gh-deploy`, or build + the official
`actions/deploy-pages`). A push to `main`/`master` touching `docs/user/**` or `mkdocs.yml` rebuilds and
publishes.

**Constraint:** GitHub Pages for **private** repositories requires **GitHub Pro/Team**. Until the repo
goes public we therefore **do not rely on Pages**:
- **Primary dev loop is fully local** — `mkdocs serve` renders the exact site (search included) at
  `localhost:8000`. We can author and review everything without Pages.
- The deploy **workflow is committed but effectively dormant** until Pages is enabled (it just won't
  publish anywhere visible on a private repo without Pro).
- *Optional early validation:* a throwaway **public** repo to prove the Action + Pages pipeline once,
  before the main repo is public.
- When the main repo goes public: enable Pages (Source = **GitHub Actions**), the workflow publishes to
  `https://<user>.github.io/<repo>/` (or a custom domain via a `CNAME`).

## What needs to be true "from the start"
1. `mkdocs.yml` at the repo root with `docs_dir: docs/user`, the Material theme, `search` plugin, and an
   explicit `nav:` mirroring the tree above.
2. All user content lives under `docs/user/` and uses relative links + the `assets/` folder.
3. The deploy workflow exists (`.github/workflows/docs.yml`) so going public is a one-switch step.
4. Dev docs stay out of `docs/user/` (never published).

## Implementation phases
- **P1 (this doc):** decide generator + layout + hosting. ← *here, awaiting sign-off*
- **P2 — scaffold:** add `mkdocs.yml` + `docs/user/` skeleton (empty stub pages with titles + nav),
  verify `mkdocs serve` renders + search works locally. Add a short `BUILD`/contributor note on running
  the docs site.
- **P3 — content:** fill the pages incrementally (start with getting-started + connecting +
  troubleshooting/connection, since testers need those first).
- **P4 — CI:** add the GitHub Pages workflow (dormant until public).
- **P5 — go live:** on repo public, enable Pages (Actions source), first deploy, optional custom domain.

## Decisions (locked 2026-06-25)
1. **Generator: MkDocs + Material for MkDocs.** ✅
2. **Language: English first** (DE/FR later via the i18n plugin). ✅
3. **Public site scope: user docs only** (`docs/user/`); dev docs stay internal. ✅

### Still defaulted (revisit later, not blocking)
- **Domain:** start on the `github.io` subpath; custom domain (CNAME + DNS) later if wanted.
- **Versioning:** single "latest" to start; add per-release versioning (`mike`) only if needed.

## Out of scope (for now)
- Auto-generating reference from code; API docs; a blog/news section.
- Translating existing dev docs.
- Search analytics / hosted (server-side) search.

[Material for MkDocs]: https://squidfunk.github.io/mkdocs-material/
