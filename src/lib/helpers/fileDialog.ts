// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// The file dialogs every caller uses instead of the plugin's own. iOS returns a percent-encoded
// `file://` URL the backend cannot open as a path; paths and Android `content://` URIs pass unchanged.

import {
  open as pluginOpen,
  save as pluginSave,
  type OpenDialogOptions,
  type OpenDialogReturn,
  type SaveDialogOptions,
} from '@tauri-apps/plugin-dialog';

function dialogPath(picked: string): string {
  if (!picked.startsWith('file://')) return picked;
  try {
    return decodeURIComponent(new URL(picked).pathname);
  } catch {
    return picked;
  }
}

export async function open<T extends OpenDialogOptions>(options?: T): Promise<OpenDialogReturn<T>> {
  const picked: string | string[] | null = await pluginOpen(options);
  const paths = Array.isArray(picked) ? picked.map(dialogPath) : picked === null ? null : dialogPath(picked);
  return paths as OpenDialogReturn<T>;
}

export async function save(options?: SaveDialogOptions): Promise<string | null> {
  const picked = await pluginSave(options);
  return picked === null ? null : dialogPath(picked);
}
