// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// File-dialog extension filters.
//
// Linux file pickers match extensions case-sensitively: the GTK3 backend compiles each filter into
// a `GPatternSpec` (`*.txt`) and the XDG desktop portal uses plain globs — neither offers a
// case-fold flag, and GLib's matcher understands only `*` and `?`, so a `*.[tT][xX][tT]` character
// class doesn't work either. A lowercase-only list therefore hides the uppercase `.TXT` blackbox
// logs that Configurator and FAT-formatted cards produce (issue #41). Windows and macOS match
// case-insensitively, where the extra patterns are simply redundant.

/** Expand extensions to their lower- and uppercase spellings, preserving order and deduplicating. */
export function anyCase(extensions: string[]): string[] {
  const out: string[] = [];
  for (const ext of extensions) {
    for (const variant of [ext.toLowerCase(), ext.toUpperCase()]) {
      if (!out.includes(variant)) out.push(variant);
    }
  }
  return out;
}
