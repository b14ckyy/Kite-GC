<script lang="ts">
  import { t } from 'svelte-i18n';
  import {
    activeSurveyPattern,
    exitPatternMode,
    switchShape,
    updateRectangleParams,
    updateCircleParams,
    type RectanglePatternParams,
    type CirclePatternParams,
  } from '$lib/stores/surveyPattern.svelte';
  import { get } from 'svelte/store';
  import {
    computeRectangleCorners,
    generateRectangleZigzag,
    generateRectangleLawnmower,
    generateCircleStepped,
    type SurveyWaypoint,
  } from '$lib/helpers/surveyPatterns';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';

  // Helper: untyped $t wrapper for dynamic params (svelte-i18n types are too strict)
  function _t(id: string, params?: Record<string, string>): string {
    return (get(t) as any)(id, { values: params });
  }

  let confirmDialog: ReturnType<typeof ConfirmDialog>;
  async function showDialog(title: string, message: string, buttons?: import('$lib/components/ConfirmDialog.svelte').DialogButton[]): Promise<string | null> {
    return confirmDialog.show({ title, message, buttons });
  }

  // Props
  interface Props {
    ongenerate?: () => void;
  }
  let { ongenerate }: Props = $props();

  // ── Rectangle state ───────────────────────────────────
  let rectangleParams = $state<RectanglePatternParams>({
    center: { lat: 0, lng: 0 },
    length: 400,
    width: 200,
    shapeOrientation: 90,
    baseAltitude: 50,
    baseSpeed: 15,
        targetLineSpacing: 50,
    actualLineSpacing: 50,
        turnDistance: 0,
        reverse: false,
        clockwise: true,
        startCorner: 1,
        trackOrientationEnabled: false,
        trackOrientation: 0,
        altMode: 'relative',
        userActionLineStartFlags: 0,
        userActionLineEndFlags: 0,
        userActionStartFlags: 0,
        userActionTrackFlags: 0,
        userActionEndFlags: 0,
  });

  // ── Circle state ──────────────────────────────────────
  let circleParams = $state<CirclePatternParams>({
    center: { lat: 0, lng: 0 },
    radius: 200,
    ringPoints: 10,
    shapeOrientation: 0,
    baseAltitude: 50,
    baseSpeed: 15,
    targetLineSpacing: 50,
    actualLineSpacing: 50,
    turnDistance: 0,
    reverse: false,
    clockwise: true,
    startCorner: 1,
    trackOrientationEnabled: false,
    trackOrientation: 0,
    altMode: 'relative',
    userActionLineStartFlags: 0,
    userActionLineEndFlags: 0,
    userActionStartFlags: 0,
    userActionTrackFlags: 0,
    userActionEndFlags: 0,
  });

  // Full list of supported shapes (visualization for non-Rectangle comes progressively)
  const availableShapes = [
    { value: 'rectangle', label: $t('survey.shapeRectangle') },
    { value: 'rectangle-lawnmower', label: $t('survey.shapeRectangleLawnmower') },
    { value: 'polygon', label: $t('survey.shapePolygon') },
    { value: 'polygon-lawnmower', label: $t('survey.shapePolygonLawnmower') },
    { value: 'circle', label: $t('survey.shapeCircle') },
    { value: 'spiral', label: $t('survey.shapeSpiral') },
  ] as const;

  function handleParamChange() {
    if (activeSurveyPattern.config && ['rectangle', 'rectangle-lawnmower'].includes(activeSurveyPattern.config.shape)) {
      updateRectangleParams(rectangleParams);
    }
  }

  function handleCircleParamChange() {
    if (activeSurveyPattern.config && ['circle', 'spiral'].includes(activeSurveyPattern.config.shape)) {
      updateCircleParams(circleParams);
    }
  }

  async function handleGenerate() {
    const cfg = activeSurveyPattern.config;
    if (!cfg) {
      exitPatternMode();
      return;
    }

    let wps: SurveyWaypoint[] = [];

        if (cfg.shape === 'rectangle') {
      const p = cfg.params as RectanglePatternParams;
      const segments = generateRectangleZigzag(p);
      // Only survey segments → extract start and end points in flight order
      const surveyPoints: SurveyWaypoint[] = [];
      for (const seg of segments) {
        if (seg.kind === 'survey') {
          // Push start (with lineStartFlags) and end (with lineEndFlags)
          surveyPoints.push(seg.points[0]);
          surveyPoints.push(seg.points[1]);
        }
      }
      wps = surveyPoints;
        } else if (cfg.shape === 'rectangle-lawnmower') {
      const p = cfg.params as RectanglePatternParams;
      const segments = generateRectangleLawnmower(p);
      // Survey segments contain all waypoints in flight order (no duplicates)
      const surveyPoints: SurveyWaypoint[] = [];
      for (const seg of segments) {
        if (seg.kind === 'survey') {
          // Each survey segment is a continuous path; take all points
          for (const pt of seg.points) {
            surveyPoints.push(pt);
          }
        }
      }
      wps = surveyPoints;
    } else if (cfg.shape === 'circle') {
      const p = cfg.params as CirclePatternParams;
      const segments = generateCircleStepped(p);
      for (const seg of segments) {
        if (seg.kind === 'survey') {
          for (const pt of seg.points) wps.push(pt);
        }
      }
    } else {
      // Other shapes: placeholder for now (will be implemented in later phases)
      console.log('[Pattern] Generation for', cfg.shape, 'not yet implemented — using empty set');
    }

    if (wps.length === 0) {
      exitPatternMode();
      return;
    }

    // Check 120 WP limit
    const { getTotalWpCount, MAX_WAYPOINTS_TOTAL } = await import('$lib/stores/mission');
    const currentCount = getTotalWpCount();
    const remaining = MAX_WAYPOINTS_TOTAL - currentCount;

    if (wps.length > remaining) {
      const result = await showDialog(
        'Waypoint limit exceeded',
        `Current mission has ${currentCount}/${MAX_WAYPOINTS_TOTAL} waypoints.\n` +
        `This pattern would add ${wps.length} waypoints, exceeding the limit by ${wps.length - remaining}.\n\n` +
        `Only the first ${remaining} waypoints will be added (truncated). Continue?`,
        [{ label: 'Cancel', value: 'cancel' }, { label: 'Truncate & Add', value: 'proceed', primary: true }]
      );
      if (result !== 'proceed') return;
      wps = wps.slice(0, remaining);
    }

    console.log(`[Pattern] Generating ${wps.length} waypoints for ${cfg.shape}`);

    // Convert SurveyWaypoint[] → INAV Waypoint[] and append to active mission
    try {
      const { missionAddWp, WpAction, altFromM, fromDeg } = await import('$lib/stores/mission');
      for (const swp of wps) {
        const altCm = altFromM(swp.alt);
        const speedCm = swp.speed ? Math.round(swp.speed * 100) : 0;
        // p3 bit 0 = altMode (0 = REL, 1 = AMSL), bits 1-4 = userActionFlags (UA1=bit1, UA2=bit2, etc.)
        let p3 = swp.altMode === 'amsl' ? 1 : 0;
        if (swp.userActionFlags) {
          p3 |= ((swp.userActionFlags & 0x0F) << 1);
        }
        await missionAddWp(WpAction.Waypoint, fromDeg(swp.lat), fromDeg(swp.lng), altCm, speedCm, 0, p3);
      }
      console.log(`[Pattern] Successfully added ${wps.length} waypoints`);
    } catch (e) {
      console.error('[Pattern] Failed to add waypoints:', e);
    }
    // Note: Mission planner auto-sets WP_FLAG_LAST on the last waypoint — no need to do it here

    exitPatternMode();
    ongenerate?.();
  }

  function handleCancel() {
    exitPatternMode();
  }

  // Sync rectangle params from store → local state (e.g. on enter or after map drag)
  let _syncing = false;
  $effect(() => {
    if (activeSurveyPattern.config && ['rectangle', 'rectangle-lawnmower'].includes(activeSurveyPattern.config.shape)) {
      const raw = activeSurveyPattern.config.params as RectanglePatternParams;
      const rounded = {
        ...raw,
        length: Math.round(raw.length * 10) / 10,
        width:  Math.round(raw.width  * 10) / 10,
      };
      rectangleParams = rounded;
      // Push rounded values back to store once so drag doesn't leave unrounded residue
      if (!_syncing && (rounded.length !== raw.length || rounded.width !== raw.width)) {
        _syncing = true;
        updateRectangleParams({ length: rounded.length, width: rounded.width });
        _syncing = false;
      }
    }
  });

  // Sync circle params from store → local state (e.g. on enter or after map drag)
  let _syncingCircle = false;
  $effect(() => {
    if (activeSurveyPattern.config && ['circle', 'spiral'].includes(activeSurveyPattern.config.shape)) {
      const raw = activeSurveyPattern.config.params as CirclePatternParams;
      const rounded = {
        ...raw,
        radius: Math.round(raw.radius * 10) / 10,
      };
      circleParams = rounded;
      if (!_syncingCircle && rounded.radius !== raw.radius) {
        _syncingCircle = true;
        updateCircleParams({ radius: rounded.radius });
        _syncingCircle = false;
      }
    }
  });

  console.log('[DEBUG] SurveyPatternPanel mounted');
</script>

<div class="survey-panel">
  <ConfirmDialog bind:this={confirmDialog} />

  <div class="survey-header">
    <div>
      <h4>{$t('survey.title')}</h4>
      <select 
        value={activeSurveyPattern.config?.shape || 'rectangle'}
        onchange={(e) => {
          switchShape((e.target as HTMLSelectElement).value as any);
        }}
      >
        {#each availableShapes as shapeOption}
          <option value={shapeOption.value}>{shapeOption.label}</option>
        {/each}
      </select>
    </div>
  </div>

  <div class="survey-params">
    {#if activeSurveyPattern.config?.shape === 'rectangle' || activeSurveyPattern.config?.shape === 'rectangle-lawnmower'}
      <!-- Row 1: Length + Width -->
      <div class="param-row">
        <NumberStepper label={$t('survey.length')} bind:value={rectangleParams.length} min={10} step={10} decimals={1} onchange={handleParamChange} />
        <NumberStepper label={$t('survey.width')} bind:value={rectangleParams.width} min={10} step={10} decimals={1} onchange={handleParamChange} />
      </div>

      <!-- Row 2: Line Spacing + Turn Distance -->
      <div class="param-row">
        <div class="spacing-wrapper">
          <NumberStepper label={$t('survey.lineSpacing')} bind:value={rectangleParams.targetLineSpacing} min={5} step={5} decimals={0} onchange={handleParamChange} />
          {#if rectangleParams.targetLineSpacing > 0 && rectangleParams.width > 0}
            <span class="spacing-info">{_t('survey.tracksInfo', { spacing: String(rectangleParams.actualLineSpacing.toFixed(1)), count: String(Math.ceil(rectangleParams.width / rectangleParams.targetLineSpacing)) })}</span>
          {/if}
        </div>
        <NumberStepper label={$t('survey.turnDistance')} bind:value={rectangleParams.turnDistance} min={0} step={5} decimals={0} onchange={handleParamChange} />
      </div>

      <!-- Shape Orientation (solo) -->
      <NumberStepper label={$t('survey.areaOrientation')} bind:value={rectangleParams.shapeOrientation} min={0} max={90} step={5} decimals={0} onchange={handleParamChange} />

            <!-- Row 3: Reverse toggle + Clockwise (lawnmower) + Track Orientation (zigzag) or Start Corner (lawnmower) -->
            <div class="param-row">
              <label class="toggle-row">
                <input type="checkbox" bind:checked={rectangleParams.reverse} onchange={handleParamChange} />
                <span>{$t('survey.reverse')}</span>
              </label>
              {#if activeSurveyPattern.config?.shape === 'rectangle-lawnmower'}
                <label class="toggle-row">
                  <input type="checkbox" bind:checked={rectangleParams.clockwise} onchange={handleParamChange} />
                  <span>{rectangleParams.clockwise ? $t('survey.counterClockwise') : $t('survey.clockwise')}</span>
                </label>
              {/if}
              {#if activeSurveyPattern.config?.shape === 'rectangle'}
                <label class="toggle-row">
                  <input type="checkbox" bind:checked={rectangleParams.trackOrientationEnabled} onchange={handleParamChange} />
                  <span>{$t('survey.trackOrientation')}</span>
                </label>
              {/if}
            </div>

            <!-- Track Orientation value for zigzag (only when enabled) -->
            {#if activeSurveyPattern.config?.shape === 'rectangle' && rectangleParams.trackOrientationEnabled}
              <NumberStepper label={$t('survey.trackOrientationVal')} bind:value={rectangleParams.trackOrientation} min={0} max={360} step={5} decimals={0} onchange={handleParamChange} />
            {/if}

            <!-- Start Corner for lawnmower -->
            {#if activeSurveyPattern.config?.shape === 'rectangle-lawnmower'}
              <NumberStepper label="Start Corner" bind:value={rectangleParams.startCorner} min={1} max={4} step={1} decimals={0} onchange={handleParamChange} />
            {/if}

      <!-- Row 4: Base Altitude + Base Speed -->
      <div class="param-row">
        <NumberStepper label={$t('survey.baseAlt')} bind:value={rectangleParams.baseAltitude} min={0} step={5} decimals={0} onchange={handleParamChange} />
        <NumberStepper label={$t('survey.baseSpeed')} bind:value={rectangleParams.baseSpeed} min={1} step={1} decimals={0} onchange={handleParamChange} />
      </div>

      <!-- Altitude Type dropdown -->
      <div class="param-row alt-type-row">
        <label class="alt-type-label">{$t('survey.altMode')}</label>
        <select class="alt-type-select" bind:value={rectangleParams.altMode} onchange={handleParamChange}>
          <option value="relative">{$t('survey.altModeRelative')}</option>
          <option value="amsl">{$t('survey.altModeAmsl')}</option>
          <option value="ground" disabled>{$t('survey.altModeGround')} — {$t('survey.comingSoon')}</option>
        </select>
      </div>

            <!-- User Action Trigger -->
      <div class="section-label">{$t('survey.userActionTrigger')}</div>

      {#if activeSurveyPattern.config?.shape === 'rectangle-lawnmower'}
        <!-- Lawnmower: Start / Track / End -->
        <div class="ua-grid">
          <div class="ua-col">
            <div class="ua-col-label">Start</div>
            <div class="ua-checks">
              {#each [1,2,3,4] as n}
                <label class="ua-check-item">
                  <input type="checkbox" checked={!!(rectangleParams.userActionStartFlags & (1 << (n-1)))} onchange={() => { rectangleParams.userActionStartFlags ^= (1 << (n-1)); handleParamChange(); }} />
                  <span>{n}</span>
                </label>
              {/each}
            </div>
          </div>
          <div class="ua-col">
            <div class="ua-col-label">Track</div>
            <div class="ua-checks">
              {#each [1,2,3,4] as n}
                <label class="ua-check-item">
                  <input type="checkbox" checked={!!(rectangleParams.userActionTrackFlags & (1 << (n-1)))} onchange={() => { rectangleParams.userActionTrackFlags ^= (1 << (n-1)); handleParamChange(); }} />
                  <span>{n}</span>
                </label>
              {/each}
            </div>
          </div>
          <div class="ua-col">
            <div class="ua-col-label">End</div>
            <div class="ua-checks">
              {#each [1,2,3,4] as n}
                <label class="ua-check-item">
                  <input type="checkbox" checked={!!(rectangleParams.userActionEndFlags & (1 << (n-1)))} onchange={() => { rectangleParams.userActionEndFlags ^= (1 << (n-1)); handleParamChange(); }} />
                  <span>{n}</span>
                </label>
              {/each}
            </div>
          </div>
        </div>
      {:else}
        <!-- Zigzag / default: Line Start + Line End -->
        <div class="ua-grid">
          <div class="ua-col">
            <div class="ua-col-label">{$t('survey.lineStart')}</div>
            <div class="ua-checks">
              {#each [1,2,3,4] as n}
                <label class="ua-check-item">
                  <input type="checkbox" checked={!!(rectangleParams.userActionLineStartFlags & (1 << (n-1)))} onchange={() => { rectangleParams.userActionLineStartFlags ^= (1 << (n-1)); handleParamChange(); }} />
                  <span>{n}</span>
                </label>
              {/each}
            </div>
          </div>
          <div class="ua-col">
            <div class="ua-col-label">{$t('survey.lineEnd')}</div>
            <div class="ua-checks">
              {#each [1,2,3,4] as n}
                <label class="ua-check-item">
                  <input type="checkbox" checked={!!(rectangleParams.userActionLineEndFlags & (1 << (n-1)))} onchange={() => { rectangleParams.userActionLineEndFlags ^= (1 << (n-1)); handleParamChange(); }} />
                  <span>{n}</span>
                </label>
              {/each}
            </div>
          </div>
        </div>
      {/if}
    {:else if activeSurveyPattern.config?.shape === 'circle'}

      <!-- Radius + Ring Points -->
      <div class="param-row">
        <NumberStepper label={$t('survey.radius')} bind:value={circleParams.radius} min={10} step={10} decimals={1} onchange={handleCircleParamChange} />
        <NumberStepper label={$t('survey.ringPoints')} bind:value={circleParams.ringPoints} min={4} max={100} step={1} decimals={0} onchange={handleCircleParamChange} />
      </div>

      <!-- Line Spacing + Ring Start Angle -->
      <div class="param-row">
        <div class="spacing-wrapper">
          <NumberStepper label={$t('survey.lineSpacing')} bind:value={circleParams.targetLineSpacing} min={5} step={5} decimals={0} onchange={handleCircleParamChange} />
        </div>
        <NumberStepper label={$t('survey.ringStartAngle')} bind:value={circleParams.trackOrientation} min={0} max={359} step={5} decimals={0} onchange={handleCircleParamChange} />
      </div>

      <!-- Direction + Reverse -->
      <div class="param-row">
        <label class="toggle-row">
          <input type="checkbox" bind:checked={circleParams.clockwise} onchange={handleCircleParamChange} />
          <span>{circleParams.clockwise ? $t('survey.clockwise') : $t('survey.counterClockwise')}</span>
        </label>
        <label class="toggle-row">
          <input type="checkbox" bind:checked={circleParams.reverse} onchange={handleCircleParamChange} />
          <span>{$t('survey.reverse')}</span>
        </label>
      </div>

      <!-- Base Altitude + Base Speed -->
      <div class="param-row">
        <NumberStepper label={$t('survey.baseAlt')} bind:value={circleParams.baseAltitude} min={0} step={5} decimals={0} onchange={handleCircleParamChange} />
        <NumberStepper label={$t('survey.baseSpeed')} bind:value={circleParams.baseSpeed} min={1} step={1} decimals={0} onchange={handleCircleParamChange} />
      </div>

      <!-- Altitude Type -->
      <div class="param-row alt-type-row">
        <label class="alt-type-label">{$t('survey.altMode')}</label>
        <select class="alt-type-select" bind:value={circleParams.altMode} onchange={handleCircleParamChange}>
          <option value="relative">{$t('survey.altModeRelative')}</option>
          <option value="amsl">{$t('survey.altModeAmsl')}</option>
          <option value="ground" disabled>{$t('survey.altModeGround')} — {$t('survey.comingSoon')}</option>
        </select>
      </div>

      <!-- User Action Triggers: Start / Track / End -->
      <div class="section-label">{$t('survey.userActionTrigger')}</div>
      <div class="ua-grid">
        <div class="ua-col">
          <div class="ua-col-label">Start</div>
          <div class="ua-checks">
            {#each [1,2,3,4] as n}
              <label class="ua-check-item">
                <input type="checkbox" checked={!!(circleParams.userActionStartFlags & (1 << (n-1)))} onchange={() => { circleParams.userActionStartFlags ^= (1 << (n-1)); handleCircleParamChange(); }} />
                <span>{n}</span>
              </label>
            {/each}
          </div>
        </div>
        <div class="ua-col">
          <div class="ua-col-label">Track</div>
          <div class="ua-checks">
            {#each [1,2,3,4] as n}
              <label class="ua-check-item">
                <input type="checkbox" checked={!!(circleParams.userActionTrackFlags & (1 << (n-1)))} onchange={() => { circleParams.userActionTrackFlags ^= (1 << (n-1)); handleCircleParamChange(); }} />
                <span>{n}</span>
              </label>
            {/each}
          </div>
        </div>
        <div class="ua-col">
          <div class="ua-col-label">End</div>
          <div class="ua-checks">
            {#each [1,2,3,4] as n}
              <label class="ua-check-item">
                <input type="checkbox" checked={!!(circleParams.userActionEndFlags & (1 << (n-1)))} onchange={() => { circleParams.userActionEndFlags ^= (1 << (n-1)); handleCircleParamChange(); }} />
                <span>{n}</span>
              </label>
            {/each}
          </div>
        </div>
      </div>

    {:else}
      <div class="not-implemented">
        {@html _t('survey.notImplemented', { shape: activeSurveyPattern.config?.shape ?? '?' })}
      </div>
    {/if}
  </div>

  <!-- Generate & Append + Load/Save buttons -->
  <div class="pattern-bottom-actions">
    <button class="btn-primary" onclick={handleGenerate}>
      ➕ {$t('survey.generateAppend')}
    </button>
    <div class="pattern-file-row">
      <button class="btn-ctrl btn-file" onclick={() => {}} disabled title={$t('survey.loadPatternSoon')}>📂 {$t('survey.loadPattern')}</button>
      <button class="btn-ctrl btn-file" onclick={() => {}} disabled title={$t('survey.savePatternSoon')}>💾 {$t('survey.savePattern')}</button>
    </div>
  </div>
</div>

<style>
  .survey-panel {
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .survey-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .survey-params {
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
    width: 100%;
    flex-shrink: 0;
  }

  .param-row {
    display: flex;
    gap: 12px;
    width: 100%;
    align-items: flex-start;
  }

  .param-row > :global(*) {
    flex: none;
    min-width: 0;
  }

  .toggle-row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: #ccc;
    cursor: pointer;
  }

  .toggle-row input[type="checkbox"] {
    accent-color: #37a8db;
  }

  .spacing-wrapper {
    display: inline-flex;
    flex-direction: column;
    flex-shrink: 0;
  }

  .spacing-info {
    font-size: 11px;
    color: #888;
    font-style: italic;
    padding-left: 4px;
  }

  .section-label {
    font-size: 12px;
    font-weight: 600;
    color: #888;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-top: 4px;
    width: 100%;
    border-bottom: 1px solid #333;
    padding-bottom: 2px;
  }

  .ua-grid {
    display: flex;
    gap: 20px;
    width: 100%;
  }

  .ua-col {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
  }

  .ua-col-label {
    font-size: 11px;
    color: #888;
    font-weight: 600;
    text-align: center;
  }

  .ua-checks {
    display: flex;
    gap: 6px;
    justify-content: center;
  }

  .ua-check-item {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
  }

  .ua-check-item input[type="checkbox"] {
    accent-color: #37a8db;
    margin: 0;
  }

  .ua-check-item span {
    font-size: 10px;
    color: #666;
  }

  .not-implemented {
    padding: 8px;
    color: #888;
    font-size: 12px;
    font-style: italic;
  }

  .alt-type-row {
    align-items: center;
    gap: 8px;
  }

  .alt-type-label {
    font-size: 12px;
    color: #aaa;
    white-space: nowrap;
  }

  .alt-type-select {
    padding: 3px 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 12px;
    color-scheme: dark;
    max-width: 120px;
  }

  .btn-sm { padding: 3px 8px; border: 1px solid #555; border-radius: 4px; background: #2a2a2a; color: #ccc; cursor: pointer; font-size: 13px; }
  .btn-sm:hover { background: #3a3a3a; }

  .survey-header select {
    padding: 2px 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 12px;
    color-scheme: dark;
    margin-left: 6px;
  }

  .survey-header h4 {
    margin: 0;
    font-size: 14px;
    color: #37a8db;
    display: inline;
  }

  .pattern-bottom-actions {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 4px 0;
    flex-shrink: 0;
  }

  .pattern-file-row {
    display: flex;
    gap: 4px;
  }

  .pattern-file-row .btn-ctrl {
    flex: 1;
    font-size: 11px;
    padding: 4px 4px;
  }

  .btn-primary {
    width: 100%;
    padding: 6px 12px;
    background: #1a3a5c;
    border: 1px solid #37a8db;
    border-radius: 4px;
    color: #37a8db;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
  }
  .btn-primary:hover {
    background: #37a8db;
    color: #fff;
  }

  .btn-ctrl {
    padding: 5px 6px;
    border: 1px solid #37a8db;
    border-radius: 4px;
    background: #1a3a5c;
    color: #37a8db;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .btn-ctrl:hover:not(:disabled) { background: #37a8db; color: #fff; }
  .btn-ctrl:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-ctrl.btn-file { border-color: #555; background: #2a2a2a; color: #ccc; }
  .btn-ctrl.btn-file:hover { background: #3a3a3a; }
</style>
