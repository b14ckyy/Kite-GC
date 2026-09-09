# Release support

Kite Ground Control is used in places where a ground station simply has to work — so a released
version does not stop receiving fixes the moment the next one appears. This page states what you can
rely on.

## Version numbers

Kite uses `MAJOR.MINOR.PATCH`:

| Kind | Example | What it brings |
| --- | --- | --- |
| **Feature release** | 1.1.0, 2.0.0 | New features and larger changes. A new *release line* starts here. |
| **Patch release** | 1.0.1, 1.0.2 | Bug fixes for an existing release line — nothing else. |
| **Pre-release** | 1.1.0-b1, 1.1.0-rc1 | Betas and release candidates on the way to a feature release. |

## The support window

**Every feature release keeps receiving patches until the *second* feature release after it has
shipped.** A feature release is a minor or a major version — whichever comes next counts.

For 1.0 this means:

- 1.0.x receives patches while 1.1 is in development **and for the whole life of 1.1**.
- Only when **1.2.0** ships (or 2.0.0, if that is the second release after 1.0) does 1.0.x reach its
  end of life. From then on, fixes land in 1.1.x and 1.2.x.

Two release lines are therefore maintained side by side at any time. A version you have proven in the
field stays supported long enough for its successor to go through its own round of field testing and
patching before you have to move.

| Line | Status | Patches until |
| --- | --- | --- |
| **1.0.x** | Live | 1.2.0 (or 2.0.0) ships |
| **1.1** | In development | — |

### Status tags

Each release line carries one of three tags — you will find it at the end of the version heading in
the [changelog](changelog.md):

| Tag | Meaning |
| --- | --- |
| **Live** | The current feature release. It receives patches. |
| **Maintenance** | Superseded by a newer feature release, but still inside its support window — it receives patches. |
| **EOL** | End of life. No further patches; move to a supported line. |

Pre-releases are not covered by the support window: a beta or release candidate is replaced by the
next pre-release or by the final version, never patched on its own.

## What a patch release contains

A patch release is the same version you already run, plus fixes:

- **Bug fixes**, including compatibility fixes for new firmware or operating-system versions and
  corrected translations.
- **No new features.** Those go into the next feature release.
- **No changes to your data** — file formats, the flight database and settings are read and written
  exactly as before, so updating to a patch never converts anything.

Every fix is recorded in the [changelog](changelog.md) inside the box of the feature release it
belongs to, under its patch version, so the notes for a release line stay in one place.

## Getting patch releases

Patch releases are published on the [GitHub releases page](https://github.com/b14ckyy/Kite-GC/releases)
like every other version. With **Check for updates** set to *Stable releases* in the settings, Kite
tells you at start-up when a newer stable version exists (see [Settings](reference/settings.md)).

Found a problem in a supported line? See [Reporting a problem](troubleshooting/reporting-issues.md) —
please mention the version you run, so the fix reaches your release line.
