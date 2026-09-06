<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- WidgetRenderer — the one id → component switch, shared by the desktop docks (WidgetPanel) and
     the phone grid (PhoneWidgetPanel). `sizePx` is the cross-axis size every square widget renders
     from; the wide ones take `wPx` × `hPx`. -->
<script lang="ts">
  import type { TelemetryData } from '$lib/stores/telemetry';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import AHI from './widgets/AHI.svelte';
  import SpeedWidget from './widgets/SpeedWidget.svelte';
  import AltWidget from './widgets/AltWidget.svelte';
  import BatteryWidget from './widgets/BatteryWidget.svelte';
  import GpsWidget from './widgets/GpsWidget.svelte';
  import RcLinkWidget from './widgets/RcLinkWidget.svelte';
  import CompassWidget from './widgets/CompassWidget.svelte';
  import HomeWidget from './widgets/HomeWidget.svelte';
  import FlightModeWidget from './widgets/FlightModeWidget.svelte';
  import LiveAglWidget from './widgets/LiveAglWidget.svelte';
  import TerrainRadarWidget from './widgets/TerrainRadarWidget.svelte';
  import VideoWidget from './widgets/VideoWidget.svelte';

  let {
    id,
    telem,
    interfaceSettings,
    sizePx,
    wPx,
    hPx,
    editing = false,
    ghost = false,
  }: {
    id: string;
    telem: TelemetryData;
    interfaceSettings: InterfaceSettings;
    /** Cross-axis size (the square widgets' edge). */
    sizePx: number;
    /** Box of the wide widgets (Live AGL, Video). */
    wPx: number;
    hPx: number;
    editing?: boolean;
    /** A visual copy (the phone grid's drag ghost): no side effects — the video widget must not
     *  register a native surface or publish its rect from here. */
    ghost?: boolean;
  } = $props();
</script>

{#if id === 'ahi'}
  <AHI {telem} size={sizePx} />
{:else if id === 'speed'}
  <SpeedWidget {telem} size={sizePx} {interfaceSettings} />
{:else if id === 'altitude'}
  <AltWidget {telem} size={sizePx} {interfaceSettings} />
{:else if id === 'battery'}
  <BatteryWidget {telem} size={sizePx} widgetId="battery" />
{:else if id === 'battery2'}
  <BatteryWidget {telem} size={sizePx} widgetId="battery2" />
{:else if id === 'gps'}
  <GpsWidget {telem} size={sizePx} />
{:else if id === 'rcLink'}
  <RcLinkWidget {telem} size={sizePx} />
{:else if id === 'compass'}
  <CompassWidget {telem} size={sizePx} {interfaceSettings} />
{:else if id === 'home'}
  <HomeWidget {telem} size={sizePx} {interfaceSettings} />
{:else if id === 'flightMode'}
  <FlightModeWidget {telem} size={sizePx} />
{:else if id === 'liveAgl'}
  <LiveAglWidget {telem} {interfaceSettings} width={wPx} height={hPx} />
{:else if id === 'terrainRadar'}
  <TerrainRadarWidget {telem} {interfaceSettings} size={sizePx} {editing} />
{:else if id === 'videoFeed'}
  <VideoWidget width={wPx} height={hPx} {ghost} />
{/if}
