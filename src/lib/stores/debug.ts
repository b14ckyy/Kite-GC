// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { writable } from 'svelte/store';

// Runtime "developer mode": true in dev builds, or in a RELEASE build started with `--debug`
// (backend `is_debug_mode`). Set once at startup from +page.svelte. Gates the in-app Debug Monitor
// and its live tooling (e.g. the 3D Performance tab) at RUNTIME — so that code ships in release builds
// (it must not be tree-shaken behind `import.meta.env.DEV`) but stays dormant unless `--debug` is passed.
export const isDebugMode = writable<boolean>(import.meta.env.DEV);
