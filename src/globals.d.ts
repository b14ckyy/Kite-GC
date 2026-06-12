// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Build-time constants injected by Vite `define` (see vite.config.js). Declared here (ambient) so they
// are never themselves rewritten by `define`. Consumed via $lib/buildInfo.ts.
declare const __APP_VERSION__: string;
declare const __GIT_COMMIT__: string;
declare const __BUILD_DATE__: string;
