// i18n initialization — svelte-i18n setup
// Default locale: English. Translations loaded synchronously at startup.

import { register, init, getLocaleFromNavigator } from 'svelte-i18n';

register('en', () => import('./locales/en.json'));
register('de', () => import('./locales/de.json'));

export function initI18n(locale?: string) {
  init({
    fallbackLocale: 'en',
    initialLocale: locale ?? getLocaleFromNavigator()?.split('-')[0] ?? 'en',
  });
}

export const SUPPORTED_LOCALES = [
  { code: 'en', label: 'English' },
  { code: 'de', label: 'Deutsch' },
] as const;
