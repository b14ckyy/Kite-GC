<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ManualConfig.svelte — PX4 MANUAL_CONTROL mapping editor (right/config stage of RcControlPanel for the
     PX4 platform). Assign a HID axis to each of the four sticks (roll/pitch/throttle/yaw), optional
     aux1–6 continuous axes, and HID buttons → MANUAL_CONTROL button numbers (1–32; the FC maps each to an
     action per-vehicle). "Learn" binds the input you move/press the most. Edits write the working manual
     map (rcManual); the profile Save button persists it. See docs/active/MAVLINK_RC_CONTROL.md §5. -->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from 'svelte-i18n';
  import { hidSnapshot } from '$lib/stores/hid';
  import { rcManual, type ManualMap } from '$lib/stores/rcManual';

  const STICKS = ['roll', 'pitch', 'throttle', 'yaw'] as const;
  type Stick = (typeof STICKS)[number];

  const axisLabels = $derived(($hidSnapshot?.axes ?? []).map((_, i) => `A${i + 1}`));
  const buttonLabels = $derived([
    ...($hidSnapshot?.buttons ?? []).map((_, i) => `B${i + 1}`),
    ...($hidSnapshot?.hats ?? []).flatMap((_, i) => [0, 1, 2, 3].map((d) => `H${i * 4 + d + 1}`)),
  ]);

  // ── Mutators (immutable updates so subscribers re-run) ──
  function setStick(key: Stick, partial: Partial<ManualMap[Stick]>): void {
    rcManual.update((m) => ({ ...m, [key]: { ...m[key], ...partial } }));
  }
  function setAux(i: number, partial: { input?: string; invert?: boolean }): void {
    rcManual.update((m) => {
      const aux = m.aux.map((a, idx) => (idx === i ? { ...a, ...partial } : a));
      return { ...m, aux };
    });
  }
  function addAux(): void {
    rcManual.update((m) => (m.aux.length >= 6 ? m : { ...m, aux: [...m.aux, { input: axisLabels[0] ?? '', invert: false }] }));
  }
  function removeAux(i: number): void {
    rcManual.update((m) => ({ ...m, aux: m.aux.filter((_, idx) => idx !== i) }));
  }
  function setButton(i: number, partial: { input?: string; button?: number }): void {
    rcManual.update((m) => {
      const buttons = m.buttons.map((b, idx) => (idx === i ? { ...b, ...partial } : b));
      return { ...m, buttons };
    });
  }
  function addButton(): void {
    rcManual.update((m) => {
      const used = new Set(m.buttons.map((b) => b.button));
      let n = 1;
      while (used.has(n) && n < 32) n++;
      return { ...m, buttons: [...m.buttons, { input: buttonLabels[0] ?? '', button: n }] };
    });
  }
  function removeButton(i: number): void {
    rcManual.update((m) => ({ ...m, buttons: m.buttons.filter((_, idx) => idx !== i) }));
  }

  // ── Learn: bind the input with the largest significant change since arming (mirrors ChannelConfig) ──
  type LearnTarget =
    | { kind: 'stick'; key: Stick }
    | { kind: 'aux'; i: number }
    | { kind: 'button'; i: number };
  let learn = $state<LearnTarget | null>(null);
  let baseAxes: number[] = [];
  let baseBtns: boolean[] = [];

  function armLearn(target: LearnTarget): void {
    if (learnEq(learn, target)) { learn = null; return; }
    baseAxes = ($hidSnapshot?.axes ?? []).map((a) => a.value);
    baseBtns = ($hidSnapshot?.buttons ?? []).map((b) => b.pressed);
    learn = target;
  }
  function learnEq(a: LearnTarget | null, b: LearnTarget): boolean {
    if (!a || a.kind !== b.kind) return false;
    if (a.kind === 'stick' && b.kind === 'stick') return a.key === b.key;
    if (a.kind === 'aux' && b.kind === 'aux') return a.i === b.i;
    if (a.kind === 'button' && b.kind === 'button') return a.i === b.i;
    return false;
  }
  const isAxisLearn = (l: LearnTarget) => l.kind === 'stick' || l.kind === 'aux';

  function finishLearn(label: string): void {
    if (!learn) return;
    if (learn.kind === 'stick') setStick(learn.key, { input: label });
    else if (learn.kind === 'aux') setAux(learn.i, { input: label });
    else setButton(learn.i, { input: label });
    learn = null;
  }

  $effect(() => {
    const snap = $hidSnapshot;
    untrack(() => {
      if (!learn || !snap) return;
      if (isAxisLearn(learn)) {
        let bestIdx = -1;
        let best = 0.3;
        snap.axes.forEach((a, i) => {
          const d = Math.abs(a.value - (baseAxes[i] ?? a.value));
          if (d > best) { best = d; bestIdx = i; }
        });
        if (bestIdx >= 0) finishLearn(`A${bestIdx + 1}`);
      } else {
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
</script>

<div class="mc">
  <!-- Four sticks -->
  <div class="mc-group">{$t('rc.manual.sticks')}</div>
  {#each STICKS as key (key)}
    {@const cfg = $rcManual[key]}
    <div class="mc-stick">
      <span class="mc-name">{$t(`rc.manual.${key}`)}</span>
      <select class="mc-input" value={cfg.input} onchange={(e) => setStick(key, { input: (e.currentTarget as HTMLSelectElement).value })}>
        <option value="">—</option>
        {#each axisLabels as lbl (lbl)}<option value={lbl}>{lbl}</option>{/each}
      </select>
      <button class="mc-learn" class:armed={learnEq(learn, { kind: 'stick', key })} onclick={() => armLearn({ kind: 'stick', key })}>
        {learnEq(learn, { kind: 'stick', key }) ? $t('rc.learning') : $t('rc.learn')}
      </button>
      <label class="mc-inv" title={$t('rc.invert')}>
        <input type="checkbox" checked={cfg.invert} onchange={(e) => setStick(key, { invert: (e.currentTarget as HTMLInputElement).checked })} />
        {$t('rc.invert')}
      </label>
    </div>
  {/each}

  <!-- Aux axes -->
  <div class="mc-group">{$t('rc.manual.aux')}</div>
  {#if $rcManual.aux.length === 0}
    <div class="mc-hint">{$t('rc.manual.noAux')}</div>
  {/if}
  {#each $rcManual.aux as a, i (i)}
    <div class="mc-stick">
      <span class="mc-name">AUX{i + 1}</span>
      <select class="mc-input" value={a.input} onchange={(e) => setAux(i, { input: (e.currentTarget as HTMLSelectElement).value })}>
        <option value="">—</option>
        {#each axisLabels as lbl (lbl)}<option value={lbl}>{lbl}</option>{/each}
      </select>
      <button class="mc-learn" class:armed={learnEq(learn, { kind: 'aux', i })} onclick={() => armLearn({ kind: 'aux', i })}>
        {learnEq(learn, { kind: 'aux', i }) ? $t('rc.learning') : $t('rc.learn')}
      </button>
      <label class="mc-inv" title={$t('rc.invert')}>
        <input type="checkbox" checked={a.invert} onchange={(e) => setAux(i, { invert: (e.currentTarget as HTMLInputElement).checked })} />
        {$t('rc.invert')}
      </label>
      <button class="mc-del" title={$t('rc.remove')} onclick={() => removeAux(i)}>✕</button>
    </div>
  {/each}
  {#if $rcManual.aux.length < 6}
    <button class="mc-add" onclick={addAux}>+ {$t('rc.manual.addAux')}</button>
  {/if}

  <!-- Buttons -->
  <div class="mc-group">{$t('rc.manual.buttons')}</div>
  <div class="mc-hint">{$t('rc.manual.buttonsHint')}</div>
  {#each $rcManual.buttons as b, i (i)}
    <div class="mc-stick">
      <select class="mc-input" value={b.input} onchange={(e) => setButton(i, { input: (e.currentTarget as HTMLSelectElement).value })}>
        <option value="">—</option>
        {#each buttonLabels as lbl (lbl)}<option value={lbl}>{lbl}</option>{/each}
      </select>
      <button class="mc-learn" class:armed={learnEq(learn, { kind: 'button', i })} onclick={() => armLearn({ kind: 'button', i })}>
        {learnEq(learn, { kind: 'button', i }) ? $t('rc.learning') : $t('rc.learn')}
      </button>
      <span class="mc-btnlbl">{$t('rc.manual.button')}</span>
      <input
        class="mc-btnnum"
        type="number"
        min="1"
        max="32"
        value={b.button}
        onchange={(e) => setButton(i, { button: Math.max(1, Math.min(32, Number((e.currentTarget as HTMLInputElement).value) || 1)) })}
      />
      <button class="mc-del" title={$t('rc.remove')} onclick={() => removeButton(i)}>✕</button>
    </div>
  {/each}
  <button class="mc-add" onclick={addButton}>+ {$t('rc.manual.addButton')}</button>
</div>

<style>
  .mc { display: flex; flex-direction: column; gap: 6px; }
  .mc-group {
    color: #37a8db; font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px;
    margin: 8px 0 2px; padding-bottom: 3px; border-bottom: 1px solid rgba(55, 168, 219, 0.25);
  }
  .mc-group:first-child { margin-top: 0; }
  .mc-hint { color: #949494; font-size: 11px; font-style: italic; }
  .mc-stick { display: flex; align-items: center; gap: 6px; }
  .mc-name { color: #cfcfcf; font-size: 11px; width: 64px; flex: none; }
  .mc-input {
    flex: 1; min-width: 0; padding: 4px 6px; font-size: 12px; background: #2a2a2a; color: #e0e0e0;
    border: 1px solid #444; border-radius: 4px;
  }
  .mc-learn {
    flex: none; padding: 4px 10px; font-size: 11px; border-radius: 4px; cursor: pointer;
    background: #2a2a2a; color: #cfcfcf; border: 1px solid #444; white-space: nowrap;
  }
  .mc-learn:hover { border-color: #37a8db; color: #37a8db; }
  .mc-learn.armed { background: rgba(55, 168, 219, 0.2); color: #37a8db; border-color: #37a8db; }
  .mc-inv {
    flex: none; display: flex; align-items: center; gap: 4px; color: #949494; font-size: 10px; white-space: nowrap;
  }
  .mc-inv input { accent-color: #37a8db; }
  .mc-btnlbl { flex: none; color: #949494; font-size: 10px; }
  .mc-btnnum {
    width: 48px; flex: none; padding: 4px 6px; font-size: 12px; background: #2a2a2a; color: #e0e0e0;
    border: 1px solid #444; border-radius: 4px; font-variant-numeric: tabular-nums;
  }
  .mc-del {
    flex: none; width: 24px; height: 24px; font-size: 11px; line-height: 1; border-radius: 4px; cursor: pointer;
    background: #2a2a2a; color: #c97070; border: 1px solid #5a3030;
  }
  .mc-del:hover { background: #3a2020; color: #e08080; }
  .mc-add {
    align-self: flex-start; padding: 4px 10px; font-size: 11px; border-radius: 4px; cursor: pointer;
    background: #2a2a2a; color: #cfcfcf; border: 1px solid #444;
  }
  .mc-add:hover { border-color: #37a8db; color: #37a8db; }
</style>
