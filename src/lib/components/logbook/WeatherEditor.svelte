<script lang="ts">
  import { t } from 'svelte-i18n';

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
</script>

<div class="weather-editor">
  <div class="weather-fields">
    <label class="weather-field">
      <span class="weather-field-label">{$t('logbook.weatherTemp')}</span>
      <div class="setting-stepper">
        <button class="stepper-btn" onclick={() => { weatherTempC = String(Math.round((Number(weatherTempC || 0) - 0.5) * 10) / 10); }}>−</button>
        <input type="number" step="0.5" class="stepper-input" bind:value={weatherTempC} placeholder="—" />
        <button class="stepper-btn" onclick={() => { weatherTempC = String(Math.round((Number(weatherTempC || 0) + 0.5) * 10) / 10); }}>+</button>
        <span class="setting-unit">{tempUnitLabel}</span>
      </div>
    </label>
    <label class="weather-field">
      <span class="weather-field-label">{$t('logbook.weatherWind')}</span>
      <div class="setting-stepper">
        <button class="stepper-btn" onclick={() => { weatherWindMs = String(Math.max(0, Math.round((Number(weatherWindMs || 0) - 0.5) * 10) / 10)); }}>−</button>
        <input type="number" step="0.5" min="0" class="stepper-input" bind:value={weatherWindMs} placeholder="—" />
        <button class="stepper-btn" onclick={() => { weatherWindMs = String(Math.round((Number(weatherWindMs || 0) + 0.5) * 10) / 10); }}>+</button>
        <span class="setting-unit">{windUnitLabel}</span>
      </div>
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

  .setting-stepper {
    display: flex;
    align-items: stretch;
    gap: 4px;
  }

  .stepper-btn {
    background: #333;
    color: #aaa;
    border: 1px solid #555;
    border-radius: 3px;
    width: 24px;
    cursor: pointer;
    font-size: 14px;
    font-weight: bold;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    user-select: none;
  }

  .stepper-btn:hover {
    background: #37a8db;
    color: #fff;
  }

  .stepper-btn:active {
    background: #2d8ab8;
  }

  .stepper-input {
    padding: 3px 4px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 11px;
    width: 52px;
    text-align: center;
    color-scheme: dark;
    appearance: textfield;
    -moz-appearance: textfield;
  }

  .stepper-input::-webkit-inner-spin-button,
  .stepper-input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .setting-unit {
    font-size: 11px;
    color: #888;
    margin-left: 2px;
    align-self: center;
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
