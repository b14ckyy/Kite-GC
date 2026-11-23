<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Dev-only widget gallery: `?id=<widget>&size=S|L|W` renders one slot from fixture data for
     tools/helpers/widget-shots.mjs, no query shows every widget in every size. Plain English, not $t(): never shipped. -->
<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { locale } from 'svelte-i18n';
  import WidgetRenderer from '$lib/components/WidgetRenderer.svelte';
  import { WIDGET_DEFS, WIDGET_MAP, sizeStates, LARGE_BASE_VMIN, SMALL_BASE_VMIN, type WidgetSize } from '$lib/config/widgetRegistry';
  import { homePosition } from '$lib/stores/home';
  import { videoState } from '$lib/stores/video';
  import { canvasSink } from '$lib/controllers/mjpegSink';
  import { FIXTURE_HOME, FIXTURE_UNITS, fixtureFrames, videoTestCard } from './widgetFixtures';

  const LARGE_PX = 240;
  const MARGIN_PX = 6;

  const params = new URLSearchParams(location.search);
  const id = params.get('id');
  const def = id ? WIDGET_MAP.get(id) : undefined;
  const size = params.get('size') as WidgetSize | null;
  const valid = !!def && !!size && sizeStates(def.shape).includes(size);

  // The box a horizontal dock gives the slot at scale 1 (WidgetPanel: unitsFor / crossFactor).
  function box(s: WidgetSize): { sizePx: number; wPx: number; hPx: number } {
    const sizePx = s === 'S' ? LARGE_PX * (SMALL_BASE_VMIN / LARGE_BASE_VMIN) : LARGE_PX;
    return { sizePx, wPx: s === 'W' ? sizePx * 2 : sizePx, hPx: sizePx };
  }

  const frames = fixtureFrames();
  let telem = $state(frames[0]);

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const nextFrame = () => new Promise((r) => requestAnimationFrame(() => r(null)));

  onMount(() => {
    if (!valid) return;
    let alive = true;
    void (async () => {
      // One frame at a time, so each widget's async terrain update finishes before the next frame arrives.
      for (const f of frames) {
        telem = f;
        await sleep(20);
      }
      await tick();
      await document.fonts.ready;
      await Promise.all([...document.images].map((img) => img.decode().catch(() => undefined)));
      const finite = document.getAnimations().filter((a) => a.effect?.getComputedTiming().iterations !== Infinity);
      await Promise.all(finite.map((a) => a.finished.catch(() => undefined)));
      await nextFrame();
      await nextFrame();
      if (alive) document.documentElement.dataset.widgetReady = '1';
    })();
    return () => {
      alive = false;
    };
  });

  if (valid) {
    locale.set('en');
    homePosition.set(FIXTURE_HOME);
    // The <img> sink takes a data URL; the off-thread canvas reader would try to fetch it as an MJPEG stream.
    canvasSink.set(false);
    videoState.update((s) => ({ ...s, status: 'live', mjpegUrl: videoTestCard(), nativeSink: false, reconnecting: false }));
  }
</script>

{#if valid && def && size}
  {@const b = box(size)}
  <div class="shot" style="padding: {MARGIN_PX}px;">
    <div class="widget-slot">
      <div class="widget-content">
        <WidgetRenderer id={def.id} {telem} interfaceSettings={FIXTURE_UNITS} sizePx={b.sizePx} wPx={b.wPx} hPx={b.hPx} small={size === 'S'} />
      </div>
    </div>
  </div>
{:else if id}
  <p class="error">Unknown widget or size: {id} / {size}</p>
{:else}
  <div class="index">
    {#each WIDGET_DEFS as w (w.id)}
      <section data-widget={w.id} data-sizes={sizeStates(w.shape).join(',')} data-default={w.defaultSize}>
        <h2>{w.label} <span>{w.id}</span></h2>
        <div class="row">
          {#each sizeStates(w.shape) as s (s)}
            {@const b = box(s)}
            <figure>
              <!-- Own document per shot: some widgets use fixed SVG ids that collide between instances. -->
              <iframe title="{w.id} {s}" src="?id={w.id}&size={s}" width={b.wPx + 2 * MARGIN_PX} height={b.hPx + 2 * MARGIN_PX}></iframe>
              <figcaption>{s}{s === w.defaultSize ? ' (default)' : ''}</figcaption>
            </figure>
          {/each}
        </div>
      </section>
    {/each}
  </div>
{/if}

<style>
  :global(body) {
    margin: 0;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    background-color: #3d3f3e;
    color: #e0e0e0;
    user-select: none;
  }
  .shot {
    display: inline-flex;
  }
  .widget-slot {
    position: relative;
  }
  .index {
    padding: 12px 16px;
  }
  h2 {
    margin: 14px 0 6px;
    font-size: 15px;
    color: #37a8db;
  }
  h2 span {
    color: #949494;
    font-weight: 400;
  }
  .row {
    display: flex;
    align-items: flex-end;
    gap: 16px;
  }
  figure {
    margin: 0;
  }
  iframe {
    display: block;
    border: none;
  }
  figcaption {
    font-size: 12px;
    color: #949494;
  }
  .error {
    padding: 16px;
    color: #d40000;
  }
</style>
