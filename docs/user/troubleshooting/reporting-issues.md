# Reporting a problem

Found a bug, or something not working as described? Reports are very welcome — a good one gets it fixed
much faster. Kite is developed openly on GitHub:

[:material-github: Open an issue on GitHub](https://github.com/b14ckyy/Kite-GC/issues){ .md-button }

## Before you file

- **Check the troubleshooting pages** — many common snags are covered under
  **[Connection](connection.md)** and **[Video](video.md)**.
- **Search existing issues.** Someone may have already reported it; adding your details to an open issue
  is more useful than a duplicate. If it's there, a 👍 or a comment with your specifics helps.
- **Try to reproduce it.** A problem you can trigger on demand is far easier to fix than a one-off.

## What to include

The more of this you provide, the quicker it can be diagnosed:

1. **A clear description** — what went wrong, in one or two sentences.
2. **Expected vs. actual** — what you thought would happen, and what actually did.
3. **Steps to reproduce** — numbered, from a known starting point, so someone else can follow them:
   ```
   1. Connect to INAV over USB serial
   2. Open the Mission planner
   3. Click "Download" …
   4. → the app shows X instead of Y
   ```
4. **Your environment:**
    - **Kite version** (shown in the top bar, and under **About** in Settings).
    - **Operating system** and version (Windows / Linux, and which).
    - **Autopilot and firmware version** — INAV / ArduPilot / PX4 and the exact version.
    - **Connection type** — USB serial, Bluetooth (SPP / BLE), TCP or UDP.
5. **Screenshots or a short screen recording** — especially for anything visual (map, widgets, 3D).
6. **The diagnostic log** — see below.

## Attach a diagnostic log

The log usually pinpoints the cause. To capture a good one:

1. Open **Settings → Diagnostics** and set **Log Level** to **Debug**.
2. **Reproduce** the problem.
3. Click **Open Log Folder** and attach the log file to the issue.

The log locations and what the file contains (port names and firmware strings, but no personal data) are
listed under **[Connection → Getting a diagnostic log](connection.md#getting-a-diagnostic-log)**.

!!! tip "One problem per issue"
    If you've hit several unrelated things, file them separately — each gets tracked and fixed on its own.
