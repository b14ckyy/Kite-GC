// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// A radio in Bluetooth trainer mode was connected as the telemetry link: the backend closed the link
// (`ble-trainer-detected`); offer the radio as the RC control input instead.

import { get } from 'svelte/store';
import { listen } from '@tauri-apps/api/event';
import { t } from 'svelte-i18n';
import type { DialogOptions } from '$lib/components/ConfirmDialog.svelte';
import { connection } from '$lib/stores/connection';
import { settings } from '$lib/stores/settings';
import { rememberTrainer, startTrainer, trainerRadioId } from '$lib/stores/trainer';
import { hidActive } from '$lib/stores/hid';
import { pushLocalStatus } from '$lib/stores/statusText';

/** The link teardown runs first; its own dialogs must not collide with this one. */
function linkClosed(timeoutMs = 3000): Promise<void> {
  return new Promise((resolve) => {
    let unsub = () => {};
    const done = () => {
      clearTimeout(timer);
      unsub();
      resolve();
    };
    const timer = setTimeout(done, timeoutMs);
    unsub = connection.subscribe((c) => {
      if (c.status === 'disconnected') queueMicrotask(done);
    });
  });
}

export async function initTrainerHandover(
  ask: (opts: DialogOptions) => Promise<string | null>,
): Promise<void> {
  await listen<{ id: string; name: string }>('ble-trainer-detected', async (e) => {
    const { id, name } = e.payload;
    await linkClosed();
    const tr = get(t);
    const title = tr('rc.trainer.detectedTitle');
    const rc = get(settings).rcControl;
    if (rc.enabled && rc.trainerRadios?.some((r) => r.id === id)) {
      // A known radio switches over without a question.
      pushLocalStatus(6, tr('rc.trainer.detectedKnown', { values: { name } }));
      if (get(hidActive) && trainerRadioId() == null) await startTrainer();
      return;
    }
    if (!rc.enabled) {
      await ask({ title, message: tr('rc.trainer.detectedInfo', { values: { name } }) });
      return;
    }
    const answer = await ask({
      title,
      message: tr('rc.trainer.detectedAsk', { values: { name } }),
      buttons: [{ label: tr('rc.trainer.detectedUse'), value: 'use', primary: true }],
    });
    if (answer === 'use') await rememberTrainer({ id, name });
  });
}
