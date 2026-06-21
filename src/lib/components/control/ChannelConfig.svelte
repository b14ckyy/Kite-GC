<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ChannelConfig.svelte — channel-centric RC mapping (right/config stage of RcControlPanel).
     All 7 methods. Inputs shown as A1/B1/H1 (not raw codes); "Learn" binds the input you move/press the
     most (significant change — works for sliders resting at an end too). Axis slots learn axes; button
     slots learn buttons/hats. No expo (firmware does that). Edits write the working channel map
     (currentChannels); the profile Save button persists them. See RC_CONTROL.md §7. -->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from 'svelte-i18n';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import { hidSnapshot } from '$lib/stores/hid';
  import { currentChannels } from '$lib/stores/rcProfiles';
  import { channelValues } from '$lib/stores/rcEngine';
  import { rcLayout } from '$lib/stores/rcLayout';
  import { rcFcConfig } from '$lib/stores/rcFcConfig';
  import { boxName, boxCategory } from '$lib/helpers/inavModes';
  import {
    defaultMethod,
    evenPositions,
    type RcMethod,
    type RcMethodKind,
  } from '$lib/helpers/rcMethods';

  const AXIS_KINDS: RcMethodKind[] = ['passthrough', 'analogAdjust', 'dualAxis'];
  const BUTTON_KINDS: RcMethodKind[] = ['hold', 'toggle', 'buttonStep', 'buttonAdjust'];

  interface Slot {
    key: string; // config field name
    type: 'axis' | 'button';
    label: string;
    value: string;
  }

  let expanded = $state<number | null>(null);
  /** Bound numeric param for the expanded channel (toggle position count / step count). */
  let stepperVal = $state(0);
  /** Learn target: which channel + config field + input type to bind next. */
  let learn = $state<{ ch: number; key: string; type: 'axis' | 'button' } | null>(null);
  let baseAxes: number[] = [];
  let baseBtns: boolean[] = [];
  let addCh = $state<number>(0);

  const channels = $derived(Object.keys($currentChannels).map(Number).sort((a, b) => a - b));
  const rawCh = $derived(channels.filter((c) => c <= $rcLayout.rawMax));
  const auxCh = $derived(channels.filter((c) => c > $rcLayout.rawMax));
  // Free channels respect the layout: split → CH1–32, single block → CH1–rawMax (16).
  const maxChannel = $derived($rcLayout.split ? $rcLayout.auxMax : $rcLayout.rawMax);
  const freeChannels = $derived(
    Array.from({ length: maxChannel }, (_, i) => i + 1).filter((n) => !(n in $currentChannels)),
  );
  const axisLabels = $derived(($hidSnapshot?.axes ?? []).map((_, i) => `A${i + 1}`));
  const buttonLabels = $derived([
    ...($hidSnapshot?.buttons ?? []).map((_, i) => `B${i + 1}`),
    ...($hidSnapshot?.hats ?? []).flatMap((_, i) => [0, 1, 2, 3].map((d) => `H${i * 4 + d + 1}`)),
  ]);

  // ── Slots per method (the input fields it needs) ──
  function slots(cfg: RcMethod): Slot[] {
    switch (cfg.kind) {
      case 'passthrough':
      case 'analogAdjust':
        return [{ key: 'input', type: 'axis', label: $t('rc.source'), value: cfg.input }];
      case 'dualAxis':
        return [
          { key: 'inputAdd', type: 'axis', label: $t('rc.sourceAdd'), value: cfg.inputAdd },
          { key: 'inputSub', type: 'axis', label: $t('rc.sourceSub'), value: cfg.inputSub },
        ];
      case 'hold':
      case 'toggle':
        return [{ key: 'input', type: 'button', label: $t('rc.source'), value: cfg.input }];
      case 'buttonStep':
      case 'buttonAdjust':
        return [
          { key: 'inputUp', type: 'button', label: $t('rc.sourceUp'), value: cfg.inputUp },
          { key: 'inputDown', type: 'button', label: $t('rc.sourceDown'), value: cfg.inputDown },
        ];
    }
  }

  function setMethod(ch: number, cfg: RcMethod): void {
    currentChannels.update((m) => ({ ...m, [ch]: cfg }));
  }
  function patch(ch: number, partial: Record<string, unknown>): void {
    currentChannels.update((m) => {
      const cur = m[ch];
      if (!cur) return m;
      return { ...m, [ch]: { ...cur, ...partial } as RcMethod };
    });
  }
  function remove(ch: number): void {
    currentChannels.update((m) => {
      const next = { ...m };
      delete next[ch];
      return next;
    });
    if (expanded === ch) expanded = null;
    if (learn?.ch === ch) learn = null;
  }
  function addChannel(): void {
    if (!addCh) return;
    setMethod(addCh, defaultMethod('passthrough', axisLabels[0] ?? ''));
    expanded = addCh;
    armLearn(addCh, 'input', 'axis');
    addCh = 0;
  }
  function changeKind(ch: number, kind: RcMethodKind): void {
    const firstType = BUTTON_KINDS.includes(kind) ? 'button' : 'axis';
    const dflt = firstType === 'button' ? buttonLabels[0] : axisLabels[0];
    setMethod(ch, defaultMethod(kind, dflt ?? ''));
    syncStepper(ch);
    // If Learn was armed for this channel, re-arm it for the new method's first input (correct type) —
    // otherwise a learn armed for an axis would keep waiting for axis motion after switching to a
    // button method (and vice versa).
    if (learn?.ch === ch) {
      const first = slots($currentChannels[ch])[0];
      if (first) armLearn(ch, first.key, first.type);
      else learn = null;
    }
  }

  function openChannel(ch: number): void {
    if (expanded === ch) {
      expanded = null;
      return;
    }
    expanded = ch;
    syncStepper(ch);
  }
  /** Seed the bound stepper from the channel's numeric param (toggle count / step count). */
  function syncStepper(ch: number): void {
    const c = $currentChannels[ch];
    if (c?.kind === 'toggle') stepperVal = c.positions.length;
    else if (c?.kind === 'buttonStep') stepperVal = c.steps;
  }

  // ── Learn: bind the input with the largest significant change since arming ──
  function armLearn(ch: number, key: string, type: 'axis' | 'button'): void {
    baseAxes = ($hidSnapshot?.axes ?? []).map((a) => a.value);
    baseBtns = ($hidSnapshot?.buttons ?? []).map((b) => b.pressed);
    learn = { ch, key, type };
  }
  $effect(() => {
    const snap = $hidSnapshot;
    untrack(() => {
      if (!learn || !snap) return;
      if (learn.type === 'axis') {
        let bestIdx = -1;
        let best = 0.3;
        snap.axes.forEach((a, i) => {
          const d = Math.abs(a.value - (baseAxes[i] ?? a.value));
          if (d > best) { best = d; bestIdx = i; }
        });
        if (bestIdx >= 0) finishLearn(`A${bestIdx + 1}`);
      } else {
        // Button: first newly-pressed button, then hat directions.
        const bi = snap.buttons.findIndex((b, i) => b.pressed && !(baseBtns[i] ?? false));
        if (bi >= 0) { finishLearn(`B${bi + 1}`); return; }
        for (let h = 0; h < snap.hats.length; h++) {
          const { x, y } = snap.hats[h];
          const dir = y > 0 ? 0 : x > 0 ? 1 : y < 0 ? 2 : x < 0 ? 3 : -1;
          if (dir >= 0) { finishLearn(`H${h * 4 + dir + 1}`); return; }
        }
      }
    });
  });
  function finishLearn(label: string): void {
    if (learn) patch(learn.ch, { [learn.key]: label });
    learn = null;
  }

  const methodLabel = (kind: RcMethodKind) => $t(`rc.method.${kind}`);
  const usPct = (us: number) => Math.max(0, Math.min(100, ((us - 1000) / 1000) * 100));
  const summary = (cfg: RcMethod) => slots(cfg).map((s) => s.value || '—').join(' / ');
  const optionsFor = (type: 'axis' | 'button') => (type === 'axis' ? axisLabels : buttonLabels);
</script>

<div class="cc">
  {#if channels.length === 0}
    <div class="cc-empty">{$t('rc.noChannels')}</div>
  {/if}

  {#snippet chBlock(ch: number)}
    {@const cfg = $currentChannels[ch]}
    {@const us = $channelValues[ch] ?? 1500}
    {@const modes = ($rcFcConfig?.mode_ranges ?? []).filter((m) => m.channel === ch)}
    <div class="cc-ch" class:open={expanded === ch}>
      <button class="cc-row" onclick={() => openChannel(ch)}>
        <span class="cc-chnum">CH{ch}</span>
        <span class="cc-method">{methodLabel(cfg.kind)}</span>
        <span class="cc-src">{summary(cfg)}</span>
        <span class="cc-val">{us}</span>
        <div class="cc-bar"><span class="cc-bar-fill" style="width:{usPct(us)}%"></span></div>
      </button>

      {#if modes.length}
        <div class="cc-modes">
          {#each modes as m (m.permanent_id)}
            <!-- Only AUX_RC channels (CH17+) are safety-relevant: they latch and persist on GCS loss.
                 The same mode on an MSP-RC channel (CH1–16) fails safe, so it's shown neutral. -->
            <span
              class="cc-mode"
              class:crit={boxCategory(m.permanent_id) === 'critical' && ch > $rcLayout.rawMax}
              class:gps={boxCategory(m.permanent_id) === 'gps' && ch > $rcLayout.rawMax}
            >{boxName(m.permanent_id)}</span>
          {/each}
        </div>
      {/if}

      {#if expanded === ch}
        <div class="cc-edit">
          <div class="cc-field">
            <span class="cc-label">{$t('rc.name')}</span>
            <input
              class="cc-input"
              type="text"
              maxlength="24"
              value={cfg.name ?? ''}
              placeholder={`CH${ch}`}
              oninput={(e) => patch(ch, { name: (e.currentTarget as HTMLInputElement).value })}
            />
          </div>

          <div class="cc-field cc-inline">
            <span class="cc-label">{$t('rc.method.label')}</span>
            <select class="cc-input" value={cfg.kind} onchange={(e) => changeKind(ch, (e.currentTarget as HTMLSelectElement).value as RcMethodKind)}>
              <optgroup label={$t('rc.method.axisGroup')}>
                {#each AXIS_KINDS as k}<option value={k}>{methodLabel(k)}</option>{/each}
              </optgroup>
              <optgroup label={$t('rc.method.buttonGroup')}>
                {#each BUTTON_KINDS as k}<option value={k}>{methodLabel(k)}</option>{/each}
              </optgroup>
            </select>
          </div>

          <!-- Input slots -->
          {#each slots(cfg) as slot (slot.key)}
            <div class="cc-field">
              <span class="cc-label">{slot.label}</span>
              <div class="cc-source">
                <select class="cc-input" value={slot.value} onchange={(e) => patch(ch, { [slot.key]: (e.currentTarget as HTMLSelectElement).value })}>
                  {#if !slot.value}<option value="">—</option>{/if}
                  {#each optionsFor(slot.type) as lbl}<option value={lbl}>{lbl}</option>{/each}
                </select>
                <button
                  class="cc-learn"
                  class:armed={learn?.ch === ch && learn?.key === slot.key}
                  onclick={() => (learn?.ch === ch && learn?.key === slot.key ? (learn = null) : armLearn(ch, slot.key, slot.type))}
                >
                  {learn?.ch === ch && learn?.key === slot.key ? $t('rc.learning') : $t('rc.learn')}
                </button>
              </div>
            </div>
          {/each}

          <!-- Per-method params -->
          {#if cfg.kind === 'passthrough' || cfg.kind === 'analogAdjust'}
            <label class="cc-field cc-inline">
              <span class="cc-label">{$t('rc.invert')}</span>
              <input type="checkbox" checked={cfg.invert} onchange={(e) => patch(ch, { invert: (e.currentTarget as HTMLInputElement).checked })} />
            </label>
            <div class="cc-field">
              <span class="cc-label">{$t('rc.deadband')} <span class="cc-pct">{Math.round(cfg.deadband * 100)}%</span></span>
              <input type="range" min="0" max="10" step="0.1" value={cfg.deadband * 100} oninput={(e) => patch(ch, { deadband: Number((e.currentTarget as HTMLInputElement).value) / 100 })} />
            </div>
          {/if}

          {#if cfg.kind === 'analogAdjust' || cfg.kind === 'buttonAdjust' || (cfg.kind === 'dualAxis' && cfg.mode === 'adjust')}
            <div class="cc-field">
              <span class="cc-label">{$t('rc.rate')} <span class="cc-pct">{Math.round(cfg.rate * 500)} µs/s</span></span>
              <input type="range" min="0.1" max="4" step="0.1" value={cfg.rate} oninput={(e) => patch(ch, { rate: Number((e.currentTarget as HTMLInputElement).value) })} />
            </div>
          {/if}

          {#if cfg.kind === 'dualAxis'}
            <div class="cc-field cc-inline">
              <span class="cc-label">{$t('rc.mode')}</span>
              <select class="cc-input" value={cfg.mode} onchange={(e) => patch(ch, { mode: (e.currentTarget as HTMLSelectElement).value })}>
                <option value="absolute">{$t('rc.modeAbsolute')}</option>
                <option value="adjust">{$t('rc.modeAdjust')}</option>
              </select>
            </div>
          {/if}

          {#if cfg.kind === 'toggle'}
            <div class="cc-field cc-inline">
              <span class="cc-label">{$t('rc.positions')}</span>
              <NumberStepper bind:value={stepperVal} min={2} max={6} onchange={() => patch(ch, { positions: evenPositions(stepperVal) })} />
            </div>
            <label class="cc-field cc-inline">
              <span class="cc-label">{$t('rc.holdToggle')}</span>
              <input type="checkbox" checked={(cfg.holdMs ?? 0) > 0} onchange={(e) => patch(ch, { holdMs: (e.currentTarget as HTMLInputElement).checked ? 1000 : 0 })} />
            </label>
            {#if (cfg.holdMs ?? 0) > 0}
              <div class="cc-field">
                <span class="cc-label">{$t('rc.holdTime')} <span class="cc-pct">{((cfg.holdMs ?? 1000) / 1000).toFixed(1)} s</span></span>
                <input type="range" min="0.5" max="2" step="0.1" value={(cfg.holdMs ?? 1000) / 1000} oninput={(e) => patch(ch, { holdMs: Math.round(Number((e.currentTarget as HTMLInputElement).value) * 1000) })} />
              </div>
            {/if}
          {/if}

          {#if cfg.kind === 'buttonStep'}
            <div class="cc-field cc-inline">
              <span class="cc-label">{$t('rc.steps')}</span>
              <NumberStepper bind:value={stepperVal} min={3} max={15} onchange={() => patch(ch, { steps: stepperVal })} />
            </div>
          {/if}

          {#if cfg.kind === 'hold'}
            <label class="cc-field cc-inline">
              <span class="cc-label">{$t('rc.invert')}</span>
              <input type="checkbox" checked={cfg.low > cfg.high} onchange={(e) => { const inv = (e.currentTarget as HTMLInputElement).checked; patch(ch, { low: inv ? 1 : -1, high: inv ? -1 : 1 }); }} />
            </label>
          {/if}

          <div class="cc-edit-foot">
            <button class="cc-remove" onclick={() => remove(ch)}>{$t('rc.remove')}</button>
          </div>
        </div>
      {/if}
    </div>
  {/snippet}

  {#if $rcLayout.split}
    {#if rawCh.length}
      <div class="cc-group">{$t('rc.groupRaw')}</div>
      {#each rawCh as ch (ch)}{@render chBlock(ch)}{/each}
    {/if}
    {#if auxCh.length}
      <div class="cc-group">{$t('rc.groupAux')}</div>
      {#each auxCh as ch (ch)}{@render chBlock(ch)}{/each}
    {/if}
  {:else}
    {#each channels as ch (ch)}{@render chBlock(ch)}{/each}
  {/if}

  {#if freeChannels.length > 0}
    <div class="cc-add">
      <select class="cc-input" bind:value={addCh}>
        <option value={0}>{$t('rc.channel')}…</option>
        {#each freeChannels as n (n)}<option value={n}>CH{n}</option>{/each}
      </select>
      <button class="cc-add-btn" disabled={!addCh} onclick={addChannel}>+ {$t('rc.addChannel')}</button>
    </div>
  {/if}
</div>

<style>
  .cc { display: flex; flex-direction: column; gap: 4px; }
  .cc-empty { color: #949494; font-size: 12px; font-style: italic; padding: 6px 2px; }
  .cc-group {
    color: #37a8db; font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px;
    margin: 8px 0 2px; padding-bottom: 3px; border-bottom: 1px solid rgba(55, 168, 219, 0.25);
  }
  .cc-group:first-child { margin-top: 0; }

  .cc-ch { border: 1px solid #333; border-radius: 5px; background: #262626; overflow: hidden; }
  .cc-ch.open { border-color: rgba(55, 168, 219, 0.4); }
  .cc-row {
    display: grid; grid-template-columns: 42px 92px 60px 42px 1fr; align-items: center; gap: 8px;
    width: 100%; padding: 6px 9px; background: none; border: none; cursor: pointer; text-align: left;
  }
  .cc-row:hover { background: rgba(255, 255, 255, 0.03); }
  .cc-chnum { color: #37a8db; font-weight: 700; font-size: 12px; }
  .cc-method { color: #cfcfcf; font-size: 11px; }
  .cc-src { color: #949494; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cc-val { color: #e0e0e0; font-size: 11px; font-variant-numeric: tabular-nums; text-align: right; }
  .cc-bar { height: 8px; background: #1f1f1f; border: 1px solid #333; border-radius: 3px; overflow: hidden; }
  .cc-bar-fill { display: block; height: 100%; background: #37a8db; }

  .cc-modes { display: flex; flex-wrap: wrap; gap: 4px; padding: 0 9px 6px; }
  .cc-mode {
    font-size: 9.5px; padding: 1px 6px; border-radius: 3px; letter-spacing: 0.3px;
    background: #2f2f2f; color: #9a9a9a; border: 1px solid #3a3a3a;
  }
  .cc-mode.crit { background: rgba(212, 0, 0, 0.18); color: #ff8a8a; border-color: rgba(212, 0, 0, 0.45); }
  .cc-mode.gps { background: rgba(232, 163, 23, 0.16); color: #f0b443; border-color: rgba(232, 163, 23, 0.4); }

  .cc-edit { padding: 8px 10px 10px; border-top: 1px solid #333; display: flex; flex-direction: column; gap: 9px; }
  .cc-field { display: flex; flex-direction: column; gap: 4px; }
  .cc-field.cc-inline { flex-direction: row; align-items: center; gap: 8px; }
  .cc-label { color: #949494; font-size: 11px; }
  .cc-pct { color: #cfcfcf; }
  .cc-source { display: flex; gap: 6px; }
  .cc-input {
    flex: 1; min-width: 0; padding: 4px 6px; font-size: 12px; background: #2a2a2a; color: #e0e0e0;
    border: 1px solid #444; border-radius: 4px;
  }
  .cc-field input[type='range'] { width: 100%; accent-color: #37a8db; }
  .cc-field input[type='checkbox'] { accent-color: #37a8db; }

  .cc-learn {
    padding: 4px 10px; font-size: 11px; border-radius: 4px; cursor: pointer;
    background: #2a2a2a; color: #cfcfcf; border: 1px solid #444; white-space: nowrap;
  }
  .cc-learn:hover { border-color: #37a8db; color: #37a8db; }
  .cc-learn.armed { background: rgba(55, 168, 219, 0.2); color: #37a8db; border-color: #37a8db; }

  .cc-edit-foot { display: flex; justify-content: flex-end; }
  .cc-remove {
    padding: 4px 10px; font-size: 11px; border-radius: 4px; cursor: pointer;
    background: #2a2a2a; color: #c97070; border: 1px solid #5a3030;
  }
  .cc-remove:hover { background: #3a2020; color: #e08080; }

  .cc-add { display: flex; gap: 6px; margin-top: 6px; }
  .cc-add-btn {
    padding: 5px 12px; font-size: 12px; font-weight: 600; border-radius: 4px; cursor: pointer;
    background: #2a2a2a; color: #cfcfcf; border: 1px solid #444; white-space: nowrap;
  }
  .cc-add-btn:hover:not(:disabled) { border-color: #37a8db; color: #37a8db; }
  .cc-add-btn:disabled { opacity: 0.45; cursor: not-allowed; }
</style>
