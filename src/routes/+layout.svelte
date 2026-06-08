<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { initI18n } from '$lib/i18n';
  import { isLoading } from 'svelte-i18n';
  import { settings } from '$lib/stores/settings';
  import { get } from 'svelte/store';

  // Initialize i18n with the persisted locale (or browser default)
  const saved = get(settings);
  initI18n(saved.locale);
</script>

{#if $isLoading}
  <!-- Wait for locale to load before rendering the app -->
{:else}
  <slot />
{/if}
