<script lang="ts">
  // Conflict-alert banner pinned to the top of the map (Plan C / C1). Two stacked banners:
  //   • Warning (red, white text + black outline): collision alarm + evade action; minimal vehicle list.
  //   • Caution (yellow, black text): proximity advisory + detailed vehicle list + hold/increase hint.
  // Both list all affected contacts (a zone may hold several). Clicking a row selects that contact.
  import { t } from 'svelte-i18n';
  import { radarAlerts, type ContactAlert } from '$lib/controllers/radarAlerts';
  import { radarSelection } from '$lib/stores/radarTracking';
  import { contactTypeKey } from '$lib/helpers/radar3d';
  import { convertSpeed, convertAltitude, convertDistance, formatConverted } from '$lib/utils/units';
  import type { InterfaceSettings } from '$lib/stores/settings';

  let { interfaceSettings }: { interfaceSettings: InterfaceSettings } = $props();

  const warnings = $derived($radarAlerts.filter((a) => a.level === 'warning'));
  const cautions = $derived($radarAlerts.filter((a) => a.level === 'caution'));
  // Worst (nearest-CPA) warning drives the evade callout in the headline (list is sorted worst-first).
  const lead = $derived(warnings.find((a) => a.evadeBearingDeg != null) ?? warnings[0]);

  const COMPASS = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'];
  const compass = (deg: number) => COMPASS[Math.round(deg / 45) % 8];

  const fmtSpeed = (ms: number | null) =>
    ms == null ? '—' : formatConverted(convertSpeed(ms, interfaceSettings.speedUnit), 0);
  const fmtDist = (m: number | null) => {
    if (m == null) return '—';
    const d = convertDistance(m, interfaceSettings.distanceUnit);
    return formatConverted(d, d.value < 10 ? 1 : 0);
  };
  /** Signed relative altitude, e.g. "+450 m" / "−200 m" / "level". */
  const fmtRelAlt = (m: number) => {
    if (Math.abs(m) < 15) return $t('radar.alert.level');
    const a = convertAltitude(Math.abs(m), interfaceSettings.altitudeUnit);
    return `${m > 0 ? '+' : '−'}${formatConverted(a, 0)}`;
  };
  const dir = (deg: number) => `${compass(deg)} ${Math.round(deg)}°`;
  const label = (a: ContactAlert) => a.callsign?.trim() || a.vehicleId;

  function select(id: string) {
    radarSelection.update((cur) => (cur === id ? null : id));
  }
</script>

{#if warnings.length || cautions.length}
  <div class="alert-stack">
    {#if warnings.length}
      <div class="banner warning" role="alert">
        <div class="headline">
          <span class="icon">⚠</span>
          <span class="title">{$t('radar.alert.warningTitle')}</span>
          <span class="action">
            {#if lead?.evadeBearingDeg != null}
              {$t('radar.alert.warningAction', { values: { dir: compass(lead.evadeBearingDeg), deg: Math.round(lead.evadeBearingDeg) } })}
            {:else}
              {$t('radar.alert.warningActionNoDir')}
            {/if}
          </span>
        </div>
        <div class="rows">
          {#each warnings as a (a.vehicleId)}
            <button class="row warn-row" onclick={() => select(a.vehicleId)}>
              <span class="r-call">{label(a)}</span>
              <span class="r-val">{fmtSpeed(a.groundSpeedMs)}</span>
              <span class="r-val">{$t('radar.alert.cpaShort')} {fmtDist(a.missH)}</span>
            </button>
          {/each}
        </div>
      </div>
    {/if}

    {#if cautions.length}
      <div class="banner caution" role="status">
        <div class="headline">
          <span class="icon">▲</span>
          <span class="title">{$t('radar.alert.cautionTitle')}</span>
          <span class="hint">{$t('radar.alert.cautionHint')}</span>
        </div>
        <div class="rows">
          {#each cautions as a (a.vehicleId)}
            <button class="row caut-row" onclick={() => select(a.vehicleId)}>
              <span class="r-call">{label(a)}</span>
              <span class="r-val">{dir(a.bearingDeg)}</span>
              <span class="r-type">{$t(contactTypeKey(a.system, a.category))}</span>
              <span class="r-val">{fmtSpeed(a.groundSpeedMs)}</span>
              <span class="r-val">{fmtRelAlt(a.relAltM)}</span>
            </button>
          {/each}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .alert-stack {
    position: absolute;
    top: 56px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 500;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: max-content;
    max-width: min(680px, 80%);
    pointer-events: none;
  }

  .banner {
    pointer-events: auto;
    border-radius: 8px;
    padding: 8px 14px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.55);
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }

  .headline {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 6px 10px;
  }

  .icon { font-size: 18px; line-height: 1; }
  .title { font-weight: 800; letter-spacing: 0.4px; }

  .rows {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 6px;
  }

  .row {
    display: flex;
    gap: 14px;
    align-items: baseline;
    justify-content: space-between;
    width: 100%;
    text-align: left;
    background: rgba(0, 0, 0, 0.12);
    border: none;
    border-radius: 4px;
    padding: 3px 8px;
    cursor: pointer;
    font-family: inherit;
    font-variant-numeric: tabular-nums;
  }

  .r-call { font-weight: 700; min-width: 84px; }
  .r-type { flex: 1; opacity: 0.9; }
  .r-val { white-space: nowrap; }

  /* ── Caution: yellow with black text ── */
  .banner.caution {
    background: #f4c020;
    color: #1a1400;
  }
  .banner.caution .title { font-size: 18px; }
  .banner.caution .hint { font-size: 14px; font-weight: 600; opacity: 0.85; }
  .banner.caution .row { font-size: 15px; background: rgba(0, 0, 0, 0.10); }
  .banner.caution .row:hover { background: rgba(0, 0, 0, 0.20); }

  /* ── Warning: red with high-contrast white text + black outline ── */
  .banner.warning {
    background: #d40000;
    color: #ffffff;
    border: 2px solid #000;
    animation: warn-pulse 1s ease-in-out infinite;
  }
  .banner.warning .title,
  .banner.warning .action,
  .banner.warning .icon,
  .banner.warning .row {
    text-shadow:
      -1px -1px 0 #000, 1px -1px 0 #000, -1px 1px 0 #000, 1px 1px 0 #000, 0 0 3px #000;
  }
  .banner.warning .title { font-size: 21px; }
  .banner.warning .action { font-size: 16px; font-weight: 700; }
  .banner.warning .row { font-size: 16px; font-weight: 700; background: rgba(0, 0, 0, 0.22); }
  .banner.warning .row:hover { background: rgba(0, 0, 0, 0.38); }

  @keyframes warn-pulse {
    0%, 100% { box-shadow: 0 4px 16px rgba(0, 0, 0, 0.55), 0 0 0 0 rgba(212, 0, 0, 0.6); }
    50% { box-shadow: 0 4px 16px rgba(0, 0, 0, 0.55), 0 0 0 6px rgba(212, 0, 0, 0); }
  }

  @media (prefers-reduced-motion: reduce) {
    .banner.warning { animation: none; }
  }
</style>
