// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// App identity + build stamp for the About dialog. The version comes from package.json and the commit id
// is captured at build time by Vite `define` (see vite.config.js) — no manual edit per build.

export const APP_NAME = 'Kite Ground Control';
export const APP_TAGLINE = 'For INAV, ArduPilot and PX4 UAV Systems';
export const APP_VERSION = __APP_VERSION__;
export const GIT_COMMIT = __GIT_COMMIT__;
export const BUILD_DATE = __BUILD_DATE__;
export const COPYRIGHT = '© 2026 Marc Hoffmann (b14ckyy)';
export const LICENSE = 'GPL-3.0-or-later';
export const REPO_URL = 'https://github.com/b14ckyy/Kite-GC';
/** Published user docs (MkDocs site); `latest` is the released minor's docs. */
export const DOCS_URL = 'https://b14ckyy.github.io/Kite-GC/latest/';
export const PRIVACY_URL = `${DOCS_URL}privacy/`;
/** The community Discord — the permanent invite, lands in #rules. */
export const DISCORD_URL = 'https://discord.gg/3FM7EWhkg9';
