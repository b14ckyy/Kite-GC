<!-- Home widget — direction arrow, distance, bearing to home position -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import type { InterfaceSettings } from "$lib/stores/settings";
  import { homePosition, type HomePosition } from "$lib/stores/home";
  import { haversineDistance, bearing } from "$lib/utils/geo";
  import { convertDistance, formatConverted } from "$lib/utils/units";
  import { get } from "svelte/store";
  import { t } from 'svelte-i18n';

  let {
    telem,
    size = 9,
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
  }: {
    telem: TelemetryData;
    size?: number;
    interfaceSettings?: InterfaceSettings;
  } = $props();

  let home = $state<HomePosition>(get(homePosition));
  homePosition.subscribe((h) => { home = h; });

  let distance = $derived(
    home.set && telem.lastUpdate
      ? haversineDistance(telem.lat, telem.lon, home.lat, home.lon)
      : 0
  );
  let homeBearing = $derived(
    home.set && telem.lastUpdate
      ? bearing(telem.lat, telem.lon, home.lat, home.lon)
      : 0
  );
  let distanceText = $derived(() => {
    const converted = convertDistance(distance, interfaceSettings.distanceUnit);
    const digits = converted.unit === 'm' || converted.unit === 'ft' ? 0 : 1;
    return formatConverted(converted, digits);
  });
  // Arrow direction relative to aircraft heading
  let relativeAngle = $derived(
    home.set ? (homeBearing - telem.yaw + 360) % 360 : 0
  );
</script>

<div class="widget-card" style="--ws: {size}px">
  <span class="w-label">{$t('widgetLabels.home')}</span>

  {#if home.set && telem.lastUpdate}
    <!-- Direction arrow -->
    <svg viewBox="0 0 60 60" class="home-arrow">
      <g transform="rotate({relativeAngle}, 30, 30)">
        <polygon points="30,8 20,38 30,32 40,38" fill="#37a8db" />
      </g>
    </svg>
    <span class="w-dist">{distanceText()}</span>
    <span class="w-bearing">{Math.round(homeBearing)}°</span>
  {:else}
    <span class="w-nodata">{$t('widgetLabels.na')}</span>
  {/if}
</div>

<style>
  .widget-card {
    width: var(--ws);
    height: var(--ws);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.02);
    box-sizing: border-box;
    padding: calc(var(--ws) * 0.05) calc(var(--ws) * 0.06) calc(var(--ws) * 0.04);
  }
  .w-label {
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .home-arrow {
    width: calc(var(--ws) * 0.42);
    height: calc(var(--ws) * 0.42);
    margin-top: calc(var(--ws) * 0.02);
  }

  .w-dist {
    font-size: calc(var(--ws) * 0.2);
    font-weight: 700;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
  }
  .w-bearing {
    font-size: calc(var(--ws) * 0.11);
    color: #888;
    font-variant-numeric: tabular-nums;
    margin-top: calc(var(--ws) * -0.01);
  }
  .w-nodata {
    font-size: calc(var(--ws) * 0.22);
    color: #555;
    font-weight: 600;
  }
</style>
