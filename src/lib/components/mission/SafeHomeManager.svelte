<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- SafeHomeManager.svelte
     INAV safehome + fixed-wing autoland editor on the panel framework. A plain `compact` PanelShell
     swapped into the INAV mission slim panel (like MissionManager, no transition). A collapsible
     "Autoland Config" box (approach-relevant params only) on top; the 8 safehome slots below
     (expandable, with per-site approach params). Editing requires a live INAV ≥7.1 link; "Save to FC"
     sends the whole working copy as one batch + EEPROM write. See docs/active/AUTOLAND_SAFEHOME.md.

     Editing model: a local display-unit mirror (g + sm) is bound to the NumberSteppers; it re-inits
     from the loaded snapshot (safehomeConfig) and `commit()`s into safehomeWorking on every change
     (so the map overlay + dirty tracking + Save see the edits). -->
<script lang="ts">
  import { untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { invoke } from '@tauri-apps/api/core';
  import { t } from 'svelte-i18n';
  import { connection } from '$lib/stores/connection';
  import { telemetry } from '$lib/stores/telemetry';
  import { settings } from '$lib/stores/settings';
  import {
    safehomeConfig, safehomeWorking, safehomeDirty, saveSafehomeConfig, revertSafehomeWorking,
    isSafehomeEmpty, setSafehomePosition, setSafehomeEnabled, clearSafehomeSlot, DEFAULT_APPROACH_ALT_M, type SafeHomeConfig,
  } from '$lib/stores/safehome';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';

  let { onBack }: { onBack: () => void } = $props();

  let confirmDialog: ReturnType<typeof ConfirmDialog>;
  let busy = $state(false);
  let statusMessage = $state('');
  let cfgOpen = $state(false);          // collapsible Autoland Config box (collapsed by default)
  let expanded = $state<Set<number>>(new Set());

  const features = $derived($connection.fcInfo?.features ?? null);
  const connectedMsp = $derived($connection.status === 'connected' && $connection.protocolType === 'msp');
  const hasAutoland = $derived($safehomeConfig?.has_autoland ?? false);
  // Editing/saving is a live INAV ≥7.1 path (the house button only opens here, but the link can drop).
  const canEdit = $derived(connectedMsp && !!features?.autoland_config);
  const validatedHint = $derived(!!features?.autoland_config && !features?.autoland_validated);
  // Flare alt/pitch only matter with a rangefinder (no rangefinder → no flare phase). The sensor status
  // is already streamed (telemetry-sensor-status); >0 = present/configured. Hiding the flare row keeps
  // the 2-col grid tidy (it's the last row).
  const hasRangefinder = $derived(($telemetry.sensorRangefinder ?? 0) > 0);
  const fwVersion = $derived(
    features?.version ? `${features.version.major}.${features.version.minor}.${features.version.patch}` : '?',
  );

  // ── Local display-unit mirror (bound to the steppers) ───────────────────────
  interface GMirror { approachLen: number; glideAlt: number; flareAlt: number; glidePitch: number; flarePitch: number; p2t: number; }
  // Headings are edited as a positive magnitude + an "exclusive" flag. INAV encodes the sign: a
  // negative heading = exclusive (that direction only), positive = bidirectional (opposite allowed),
  // 0 = off. We invert in the background on commit so the field never shows a negative value.
  // Mirror holds only the NumberStepper/select fields. enabled + lat/lon are edited directly on the
  // working store (shared with the map drag + "+" button), so they're NOT mirrored here.
  interface SMirror { appAlt: number; landAlt: number; hdg1Mag: number; hdg1Excl: boolean; hdg2Mag: number; hdg2Excl: boolean; dir: number; seaLevel: boolean; }

  /** magnitude + excl flag → INAV's signed heading (0 stays 0 = off). */
  const signedHeading = (mag: number, excl: boolean) => {
    const m = Math.round(mag);
    return m === 0 ? 0 : (excl ? -m : m);
  };

  let g = $state<GMirror | null>(null);
  let sm = $state<SMirror[]>([]);

  const cmToM = (cm: number | null | undefined) => (cm == null ? 0 : cm / 100);

  function initMirror(c: SafeHomeConfig | null) {
    if (!c) { g = null; sm = []; return; }
    g = c.has_autoland
      ? {
          approachLen: cmToM(c.autoland.approach_length_cm),
          glideAlt: cmToM(c.autoland.glide_alt_cm),
          flareAlt: cmToM(c.autoland.flare_alt_cm),
          glidePitch: c.autoland.glide_pitch_deg ?? 0,
          flarePitch: c.autoland.flare_pitch_deg ?? 0,
          p2t: c.autoland.pitch2throttle_mod ?? 100,
        }
      : null;
    sm = c.safehomes.map((s) => {
      const ap = c.approaches.find((a) => a.index === s.index);
      return {
        appAlt: isSafehomeEmpty(s) ? DEFAULT_APPROACH_ALT_M : (ap?.approach_alt_cm ?? 0) / 100,
        landAlt: ap ? ap.land_alt_cm / 100 : 0,
        hdg1Mag: Math.abs(ap?.heading1 ?? 0),
        hdg1Excl: (ap?.heading1 ?? 0) < 0,
        hdg2Mag: Math.abs(ap?.heading2 ?? 0),
        hdg2Excl: (ap?.heading2 ?? 0) < 0,
        dir: ap?.approach_direction ?? 0,
        seaLevel: ap?.sea_level_ref ?? false,
      };
    });
  }

  /** Clean a slot: clear coords (→ unset, hidden on the map) + reset its approach, and reset the display
   *  mirror so the now-empty slot shows the default again. */
  function onClean(i: number) {
    clearSafehomeSlot(i);
    sm[i] = { appAlt: DEFAULT_APPROACH_ALT_M, landAlt: 0, hdg1Mag: 0, hdg1Excl: false, hdg2Mag: 0, hdg2Excl: false, dir: 0, seaLevel: false };
  }

  /** Push the mirror into the working copy (map overlay + dirty + Save read this). */
  function commit() {
    safehomeWorking.update((w) => {
      if (!w) return w;
      const autoland = g
        ? {
            ...w.autoland,
            approach_length_cm: Math.round(g.approachLen * 100),
            glide_alt_cm: Math.round(g.glideAlt * 100),
            flare_alt_cm: Math.round(g.flareAlt * 100),
            glide_pitch_deg: Math.round(g.glidePitch),
            flare_pitch_deg: Math.round(g.flarePitch),
            pitch2throttle_mod: Math.round(g.p2t),
          }
        : w.autoland;
      const approaches = w.approaches.map((a) => {
        const i = w.safehomes.findIndex((s) => s.index === a.index);
        const r = i >= 0 ? sm[i] : undefined;
        return r
          ? { ...a, approach_alt_cm: Math.round(r.appAlt * 100), land_alt_cm: Math.round(r.landAlt * 100), heading1: signedHeading(r.hdg1Mag, r.hdg1Excl), heading2: signedHeading(r.hdg2Mag, r.hdg2Excl), approach_direction: r.dir, sea_level_ref: r.seaLevel }
          : a;
      });
      return { ...w, autoland, approaches }; // safehomes (lat/lon/enabled) edited directly via helpers
    });
  }

  // Re-init the mirror whenever a fresh snapshot loads (connect / after Save re-read). Doesn't fire on
  // edits — safehomeConfig only changes on load, not on working-copy patches.
  $effect(() => {
    const loaded = $safehomeConfig;
    untrack(() => initMirror(loaded));
  });

  function toggleExpand(i: number) {
    const n = new Set(expanded);
    if (n.has(i)) n.delete(i);
    else n.add(i);
    expanded = n;
  }

  /** Place safehome i at the current 2D map-view centre (and enable it). Drag-refine on the map. */
  function setFromMapCenter(i: number) {
    if (!canEdit) return;
    const c = get(settings).map.center;
    setSafehomePosition(i, Math.round(c[0] * 1e7), Math.round(c[1] * 1e7), true);
    if (!expanded.has(i)) toggleExpand(i);
    commit(); // push the mirror's approach defaults (incl. the 40 m approach alt) into the working copy
  }

  /** Toggle the sea-level reference and auto-convert approach/land alt using the terrain elevation at the
   *  safehome (REL→MSL adds the ground elevation, MSL→REL subtracts it) so the physical altitude is kept.
   *  If terrain data isn't available, the flag still flips but the values are left as-is (with a note). */
  async function onSeaLevelToggle(i: number, newVal: boolean) {
    const r = sm[i];
    if (!r || r.seaLevel === newVal) return;
    const s = get(safehomeWorking)?.safehomes[i];
    let elev: number | null = null;
    if (s && !isSafehomeEmpty(s)) {
      try {
        elev = await invoke<number | null>('terrain_elevation', { lat: s.lat / 1e7, lon: s.lon / 1e7 });
      } catch {
        elev = null;
      }
    }
    if (elev != null && isFinite(elev)) {
      const sign = newVal ? 1 : -1;
      r.appAlt = Math.round(r.appAlt + sign * elev);
      r.landAlt = Math.round((r.landAlt + sign * elev) * 10) / 10;
    } else {
      statusMessage = $t('safehome.noTerrain');
    }
    r.seaLevel = newVal;
    commit();
  }

  /** Lat/Lon field edit → write straight to the working store (keeps map + panel in sync). */
  function onCoordInput(i: number, which: 'lat' | 'lon', v: string) {
    const n = Number(v);
    if (v.trim() === '' || isNaN(n)) return;
    const s = get(safehomeWorking)?.safehomes[i];
    if (!s) return;
    if (which === 'lat') setSafehomePosition(i, Math.round(n * 1e7), s.lon);
    else setSafehomePosition(i, s.lat, Math.round(n * 1e7));
  }

  function onRevert() {
    revertSafehomeWorking();
    initMirror(get(safehomeConfig));
  }

  async function onSave() {
    if (!canEdit || busy) return;
    const ans = await confirmDialog.show({
      title: $t('safehome.saveTitle'),
      message: $t('safehome.saveMsg'),
      buttons: [{ label: $t('safehome.saveConfirm'), value: 'ok', primary: true }],
    });
    if (ans !== 'ok') return;
    busy = true;
    statusMessage = '';
    try {
      await saveSafehomeConfig();
      statusMessage = $t('safehome.saved');
    } catch (e) {
      statusMessage = $t('safehome.saveFailed', { values: { error: String(e) } });
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    if (!statusMessage) return;
    const id = setTimeout(() => { statusMessage = ''; }, 8000);
    return () => clearTimeout(id);
  });
</script>

{#snippet toolbar()}
  <div class="shm-toolbar">
    <Button variant="standard" icon="home" onclick={onBack}>← {$t('safehome.back')}</Button>
    <div class="tb-spacer"></div>
    <label class="shm-show" title={$t('safehome.showOnMap')}>
      <Toggle checked={$settings.showSafehomes} id="shm-show" onchange={(c) => settings.patch({ showSafehomes: c })} />
      <span>{$t('safehome.showOnMap')}</span>
    </label>
  </div>
{/snippet}

{#snippet body()}
  <div class="shm-body">
    {#if !$safehomeConfig || !$safehomeWorking}
      <div class="shm-note">{$t('safehome.notLoaded')}</div>
    {:else}
      {#if !connectedMsp}
        <div class="shm-note warn">{$t('safehome.connectHint')}</div>
      {:else if validatedHint}
        <div class="shm-note warn">{$t('safehome.notValidated', { values: { version: fwVersion } })}</div>
      {/if}

      <!-- ── Collapsible global autoland config (approach-relevant params only) ── -->
      {#if hasAutoland && g}
        <button class="shm-collapse" onclick={() => (cfgOpen = !cfgOpen)}>
          <span class="cc-caret">{cfgOpen ? '▾' : '▸'}</span>
          <span class="cc-title">{$t('safehome.autolandConfig')}</span>
        </button>
        {#if cfgOpen}
          <!-- Ordered left→right, top→bottom: Approach | Pitch2Throttle / Glide alt | Glide pitch /
               Flare alt | Flare pitch (last row hidden without a rangefinder — no flare phase). -->
          <div class="shm-grid" class:readonly={!canEdit}>
            <div class="cell"><span class="cl">{$t('safehome.approachLength')}</span><NumberStepper bind:value={g.approachLen} min={1} max={1000} step={10} decimals={0} unit="m" disabled={!canEdit} onchange={commit} /></div>
            <div class="cell"><span class="cl">{$t('safehome.pitch2throttle')}</span><NumberStepper bind:value={g.p2t} min={100} max={400} step={5} decimals={0} unit="%" disabled={!canEdit} onchange={commit} /></div>
            <div class="cell"><span class="cl">{$t('safehome.glideAlt')}</span><NumberStepper bind:value={g.glideAlt} min={1} max={50} step={0.5} decimals={1} unit="m" disabled={!canEdit} onchange={commit} /></div>
            <div class="cell"><span class="cl">{$t('safehome.glidePitch')}</span><NumberStepper bind:value={g.glidePitch} min={0} max={45} step={1} decimals={0} unit="°" disabled={!canEdit} onchange={commit} /></div>
            {#if hasRangefinder}
              <div class="cell"><span class="cl">{$t('safehome.flareAlt')}</span><NumberStepper bind:value={g.flareAlt} min={0} max={50} step={0.1} decimals={1} unit="m" disabled={!canEdit} onchange={commit} /></div>
              <div class="cell"><span class="cl">{$t('safehome.flarePitch')}</span><NumberStepper bind:value={g.flarePitch} min={0} max={45} step={1} decimals={0} unit="°" disabled={!canEdit} onchange={commit} /></div>
            {/if}
          </div>
        {/if}
      {/if}

      <!-- ── Safehome slots ─────────────────────────────────────────── -->
      <div class="shm-section">{$t('safehome.safehomes')}</div>
      {#each sm as r, i (i)}
        {@const shw = $safehomeWorking.safehomes[i]}
        {@const empty = isSafehomeEmpty(shw)}
        <div class="sh-row" class:empty={empty && !shw.enabled}>
          <div class="sh-head">
            <button class="sh-caret" onclick={() => toggleExpand(i)} title={$t('safehome.expand')}>{expanded.has(i) ? '▾' : '▸'}</button>
            <span class="sh-badge">{i + 1}</span>
            <Toggle checked={shw.enabled} id={`sh-en-${i}`} disabled={!canEdit} onchange={(c) => setSafehomeEnabled(i, c)} />
            <input class="sh-coord" type="number" step="0.0000001" disabled={!canEdit} placeholder={$t('safehome.lat')} value={empty ? '' : (shw.lat / 1e7).toFixed(7)} onchange={(e) => onCoordInput(i, 'lat', e.currentTarget.value)} />
            <input class="sh-coord" type="number" step="0.0000001" disabled={!canEdit} placeholder={$t('safehome.lon')} value={empty ? '' : (shw.lon / 1e7).toFixed(7)} onchange={(e) => onCoordInput(i, 'lon', e.currentTarget.value)} />
            <Button variant="standard" icon="add" disabled={!canEdit} title={$t('safehome.setHere')} onclick={() => setFromMapCenter(i)} />
            <Button variant="standard" icon="delete" disabled={!canEdit || empty} title={$t('safehome.clean')} onclick={() => onClean(i)} />
          </div>
          {#if expanded.has(i)}
            {#if hasAutoland}
              <div class="sh-approach" class:readonly={!canEdit}>
                <div class="cell"><span class="cl">{$t('safehome.approachAlt')}</span><NumberStepper bind:value={r.appAlt} min={1} max={1000} step={1} decimals={0} unit="m" disabled={!canEdit} onchange={commit} /></div>
                <div class="cell"><span class="cl">{$t('safehome.landAlt')}</span><NumberStepper bind:value={r.landAlt} min={-500} max={5000} step={0.1} decimals={1} unit="m" disabled={!canEdit} onchange={commit} /></div>
                <div class="cell wide dir">
                  <span class="cl hdl">{$t('safehome.direction')}</span>
                  <span class="dir-opt" class:active={r.dir === 0}>{$t('safehome.dirLeft')}</span>
                  <Toggle checked={r.dir === 1} id={`sh-dir-${i}`} disabled={!canEdit} onchange={(c) => { r.dir = c ? 1 : 0; commit(); }} />
                  <span class="dir-opt" class:active={r.dir === 1}>{$t('safehome.dirRight')}</span>
                </div>
                <div class="cell wide hd">
                  <span class="cl hdl">{$t('safehome.heading1')}</span>
                  <NumberStepper bind:value={r.hdg1Mag} min={0} max={360} step={1} decimals={0} unit="°" disabled={!canEdit} onchange={commit} />
                  <label class="excl"><Toggle checked={r.hdg1Excl} id={`sh-x1-${i}`} disabled={!canEdit} onchange={(c) => { r.hdg1Excl = c; commit(); }} /><span>{$t('safehome.exclusive')}</span></label>
                </div>
                <div class="cell wide hd">
                  <span class="cl hdl">{$t('safehome.heading2')}</span>
                  <NumberStepper bind:value={r.hdg2Mag} min={0} max={360} step={1} decimals={0} unit="°" disabled={!canEdit} onchange={commit} />
                  <label class="excl"><Toggle checked={r.hdg2Excl} id={`sh-x2-${i}`} disabled={!canEdit} onchange={(c) => { r.hdg2Excl = c; commit(); }} /><span>{$t('safehome.exclusive')}</span></label>
                </div>
                <label class="cell wide chk"><Toggle checked={r.seaLevel} id={`sh-sl-${i}`} disabled={!canEdit} onchange={(c) => onSeaLevelToggle(i, c)} /><span>{$t('safehome.seaLevelRef')}</span></label>
              </div>
            {:else}
              <div class="sh-approach"><span class="shm-note">{$t('safehome.noApproach')}</span></div>
            {/if}
          {/if}
        </div>
      {/each}

      <div class="shm-actions">
        <Button variant="standard" icon="undo" disabled={!$safehomeDirty || busy} onclick={onRevert}>{$t('safehome.revert')}</Button>
        <Button variant="data" icon="save" disabled={!canEdit || !$safehomeDirty || busy} onclick={onSave}>{busy ? $t('safehome.saving') : $t('safehome.saveToFc')}</Button>
      </div>
      {#if statusMessage}<div class="shm-status">{statusMessage}</div>{/if}
    {/if}
  </div>
{/snippet}

<div class="shmv2">
  <PanelShell variant="compact" title={$t('safehome.title')} {toolbar} {body} />
</div>

<ConfirmDialog bind:this={confirmDialog} />

<style>
  .shm-toolbar { display: flex; align-items: center; gap: 6px; width: 100%; }
  .tb-spacer { flex: 1; }
  .shm-show { display: flex; align-items: center; gap: 6px; font-size: 11px; color: #cfcfcf; }

  .shm-body { display: flex; flex-direction: column; gap: 6px; }
  .shm-note { font-size: 12px; color: #888; padding: 4px 2px; }
  .shm-note.warn { color: #f5a623; }

  .shm-section { margin: 6px 0 2px; font-size: 11px; font-weight: 600; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px; }

  /* Collapsible header (like the Raw HID monitor box). */
  .shm-collapse { display: flex; align-items: center; gap: 6px; width: 100%; text-align: left; background: #2a2a2a; border: 1px solid #3a3a3a; border-radius: 4px; color: #cfcfcf; padding: 5px 7px; cursor: pointer; font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; }
  .shm-collapse:hover { border-color: #37a8db; }
  .cc-caret { color: #9cc6d9; font-size: 11px; }
  .cc-title { color: #37a8db; }

  /* Fixed two-column grid sized to the ~380px compact field. */
  .shm-grid, .sh-approach { display: grid; grid-template-columns: 1fr 1fr; gap: 8px 10px; padding: 6px 2px; }
  .shm-grid.readonly, .sh-approach.readonly { opacity: 0.65; }
  .cell { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .cell.wide { grid-column: span 2; }
  .cell.chk { flex-direction: row; align-items: center; gap: 8px; }
  .cell.hd { flex-direction: row; align-items: center; gap: 8px; }
  .cell.hd .hdl { width: 84px; flex: none; }
  .excl { display: flex; align-items: center; gap: 5px; font-size: 11px; color: #cfcfcf; }
  .cl { font-size: 10px; color: #949494; text-transform: uppercase; letter-spacing: 0.04em; }
  .cell.dir { flex-direction: row; align-items: center; gap: 8px; }
  .dir-opt { font-size: 11px; color: #777; }
  .dir-opt.active { color: #e0e0e0; font-weight: 600; }

  .sh-row { border: 1px solid #333; border-radius: 4px; background: #242424; padding: 4px 6px; }
  .sh-row.empty { opacity: 0.55; }
  .sh-head { display: flex; align-items: center; gap: 6px; }
  .sh-caret { background: none; border: none; color: #9cc6d9; cursor: pointer; font-size: 12px; width: 16px; flex: none; }
  .sh-badge { display: inline-flex; align-items: center; justify-content: center; width: 20px; height: 20px; border-radius: 50%; background: #37a8db; color: #fff; font-size: 10px; font-weight: bold; flex: none; }
  .sh-coord { width: 84px; flex: 1; min-width: 0; height: 24px; box-sizing: border-box; padding: 0 5px; font-size: 11px; color: #e0e0e0; background: #1f1f1f; border: 1px solid #444; border-radius: 4px; font-variant-numeric: tabular-nums; }

  .shm-actions { display: flex; gap: 6px; margin-top: 8px; }
  .shm-actions :global(.kbtn) { flex: 1; }
  .shm-status { padding: 3px 6px; font-size: 11px; color: #f39c12; text-align: center; }
</style>
