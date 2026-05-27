<script lang="ts">
  import { t } from 'svelte-i18n';
  import NumberStepper from '$lib/components/NumberStepper.svelte';

  let {
    weatherTempC = $bindable(),
    weatherWindMs = $bindable(),
    weatherWindDir = $bindable(),
    weatherDesc = $bindable(),
    tempUnitLabel,
    windUnitLabel,
    onSave,
  }: {
    weatherTempC: string;
    weatherWindMs: string;
    weatherWindDir: string;
    weatherDesc: string;
    tempUnitLabel: string;
    windUnitLabel: string;
    onSave: () => void;
  } = $props();

  const standardWindDirValues = ['0', '45', '90', '135', '180', '225', '270', '315'];
  const standardWeatherConditions = [
    'Clear', 'Partly Cloudy', 'Overcast', 'Light Rain', 'Moderate Rain',
    'Rain', 'Snow', 'Fog', 'Stormy',
  ];

  function hasStandardWeatherCondition(value: string): boolean {
    const normalized = value.trim().toLowerCase();
    return standardWeatherConditions.some((c) => c.toLowerCase() === normalized);
  }

  function windDegToLabel(deg: number): string {
    const dirs = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'];
    return dirs[Math.round(deg / 45) % 8];
  }

  // Bridge between string bindable props and NumberStepper numbers
  let weatherTempNum = $state(Number(weatherTempC || 0));
  let weatherWindNum = $state(Number(weatherWindMs || 0));
</script>

<div class="weather-editor">
  <div class="weather-fields">
    <label class="weather-field">
      <span class="weather-field-label">{$t('logbook.weatherTemp')}</span>
      <NumberStepper bind:value={weatherTempNum} step={0.5} unit={tempUnitLabel} onchange={() => { weatherTempC = String(weatherTempNum); }} />
    </label>
    <label class="weather-field">
      <span class="weather-field-label">{$t('logbook.weatherWind')}</span>
      <NumberStepper bind:value={weatherWindNum} step={0.5} min={0} unit={windUnitLabel} onchange={() => { weatherWindMs = String(weatherWindNum); }} />
    </label>
    <label class="weather-field">
      <span class="weather-field-label">{$t('logbook.weatherWindDir')}</span>
      <select class="setting-select weather-select" bind:value={weatherWindDir}>
        <option value="">—</option>
        {#if weatherWindDir && !standardWindDirValues.includes(weatherWindDir)}
          <option value={weatherWindDir}>{weatherWindDir}° ({windDegToLabel(Number(weatherWindDir))})</option>
        {/if}
        <option value="0">N</option>
        <option value="45">NE</option>
        <option value="90">E</option>
        <option value="135">SE</option>
        <option value="180">S</option>
        <option value="225">SW</option>
        <option value="270">W</option>
        <option value="315">NW</option>
      </select>
    </label>
    <label class="weather-field">
      <span class="weather-field-label">{$t('logbook.weatherConditions')}</span>
      <select class="setting-select weather-select" bind:value={weatherDesc}>
        <option value="">—</option>
        {#if weatherDesc && !hasStandardWeatherCondition(weatherDesc)}
          <option value={weatherDesc}>{weatherDesc}</option>
        {/if}
        <option value="Clear">{$t('logbook.weatherClear')}</option>
        <option value="Partly Cloudy">{$t('logbook.weatherPartlyCloudy')}</option>
        <option value="Overcast">{$t('logbook.weatherOvercast')}</option>
        <option value="Light Rain">{$t('logbook.weatherLightRain')}</option>
        <option value="Moderate Rain">{$t('logbook.weatherModerateRain')}</option>
        <option value="Rain">{$t('logbook.weatherRain')}</option>
        <option value="Snow">{$t('logbook.weatherSnow')}</option>
        <option value="Fog">{$t('logbook.weatherFog')}</option>
        <option value="Stormy">{$t('logbook.weatherStormy')}</option>
      </select>
    </label>
  </div>
  <button class="cache-clear-btn weather-save-btn" onclick={onSave}>{$t('logbook.saveWeather')}</button>
</div>

<style>
  .weather-editor {
    margin-top: 8px;
    padding: 10px;
    border: 1px solid rgba(55, 168, 219, 0.25);
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.03);
  }

  .weather-fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .weather-field {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .weather-field-label {
    font-size: 10px;
    color: #949494;
  }

  .setting-select {
    padding: 3px 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 11px;
    min-width: 70px;
  }

  .weather-select {
    width: 100%;
    box-sizing: border-box;
  }

  .weather-save-btn {
    margin-top: 8px;
    width: 100%;
  }

  .cache-clear-btn {
    font-size: 9px;
    padding: 1px 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #ccc;
    cursor: pointer;
    transition: background 0.15s;
  }

  .cache-clear-btn:hover {
    background: #c0392b;
    border-color: #c0392b;
    color: #fff;
  }
</style>
