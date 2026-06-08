// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// i18n initialization — svelte-i18n setup
// Default locale: English. Translations loaded synchronously at startup.

import { register, init, getLocaleFromNavigator } from 'svelte-i18n';

register('en', () => import('./locales/en.json'));
register('de', () => import('./locales/de.json'));
// French — experimental / not on the mandatory dual-update list (see CLAUDE.md). Based on en.json.
register('fr', () => import('./locales/fr.json'));

export function initI18n(locale?: string) {
  init({
    fallbackLocale: 'en',
    initialLocale: locale ?? getLocaleFromNavigator()?.split('-')[0] ?? 'en',
  });
}

export const SUPPORTED_LOCALES = [
  { code: 'en', label: 'English' },
  { code: 'de', label: 'Deutsch' },
  { code: 'fr', label: 'Français' },
] as const;
