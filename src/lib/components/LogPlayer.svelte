<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { Flight } from '$lib/stores/flightlog';

  let {
    showPlayer = false,
    selectedFlight = null,
    playbackPlaying = false,
    playbackSpeed = 1,
    playbackCurrentMs = 0,
    playbackTotalMs = 0,
    trackLength = 0,
    playbackIndex = 0,
    onClose = () => {},
    onSeekToStart = () => {},
    onSeek = (_deltaMs: number) => {},
    onTogglePlayPause = () => {},
    onCycleSpeed = () => {},
    onScrub = (_index: number) => {},
    onScrubStart = () => {},
    onScrubEnd = () => {},
  }: {
    showPlayer?: boolean;
    selectedFlight?: Flight | null;
    playbackPlaying?: boolean;
    playbackSpeed?: number;
    playbackCurrentMs?: number;
    playbackTotalMs?: number;
    trackLength?: number;
    playbackIndex?: number;
    onClose?: () => void;
    onSeekToStart?: () => void;
    onSeek?: (deltaMs: number) => void;
    onTogglePlayPause?: () => void;
    onCycleSpeed?: () => void;
    onScrub?: (index: number) => void;
    onScrubStart?: () => void;
    onScrubEnd?: () => void;
  } = $props();

  function formatPlaybackTime(ms: number): string {
    const totalSec = Math.floor(ms / 1000);
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  }

  function handleScrub(event: Event) {
    const target = event.currentTarget as HTMLInputElement;
    onScrub(Number(target.value));
  }
</script>

{#if showPlayer && selectedFlight}
  <div class="log-player">
    <div class="log-player-top">
      <div class="log-player-source">
        {#if selectedFlight?.source === 'blackbox'}
          <button class="log-player-source-btn" disabled>REC</button>
          <button class="log-player-source-btn active">BBX</button>
        {:else if selectedFlight?.source === 'both'}
          <button class="log-player-source-btn active">REC</button>
          <button class="log-player-source-btn active">BBX</button>
        {:else}
          <button class="log-player-source-btn active">REC</button>
          <button class="log-player-source-btn" disabled title={$t('player.bbxNotAvailable')}>BBX</button>
        {/if}
      </div>
      <div class="log-player-title">
        {selectedFlight.craft_name || $t('logbook.unknownCraft')}
        {#if selectedFlight.fc_variant || selectedFlight.fc_version}
          <span class="log-player-firmware">- {selectedFlight.fc_variant} {selectedFlight.fc_version}</span>
        {/if}
      </div>
      <button class="log-player-close" onclick={onClose} title={$t('player.close')}>X</button>
    </div>

    <div class="log-player-controls">
      <span class="log-player-time">{formatPlaybackTime(playbackCurrentMs)}</span>
      <div class="log-player-buttons">
        <button class="log-player-btn" onclick={onSeekToStart} title={$t('player.toStart')}>|&lt;</button>
        <button class="log-player-btn" onclick={() => onSeek(-300000)} title="-5min">-5m</button>
        <button class="log-player-btn" onclick={() => onSeek(-60000)} title="-1min">-1m</button>
        <button class="log-player-btn" onclick={() => onSeek(-10000)} title="-10s">-10s</button>
        <button class="log-player-btn play-btn" onclick={onTogglePlayPause} title={playbackPlaying ? $t('player.pause') : $t('player.play')}>
          {playbackPlaying ? '||' : '>'}
        </button>
        <button class="log-player-btn" onclick={() => onSeek(10000)} title="+10s">+10s</button>
        <button class="log-player-btn" onclick={() => onSeek(60000)} title="+1min">+1m</button>
        <button class="log-player-btn" onclick={() => onSeek(300000)} title="+5min">+5m</button>
        <button class="log-player-btn speed-btn" onclick={onCycleSpeed} title={$t('player.speed')}>
          {playbackSpeed}x
        </button>
      </div>
      <span class="log-player-time">{formatPlaybackTime(playbackTotalMs)}</span>
    </div>

    <div class="log-player-scrubber">
      <input
        type="range"
        min="0"
        max={Math.max(trackLength - 1, 0)}
        value={playbackIndex}
        class="log-player-slider"
        oninput={handleScrub}
        onpointerdown={onScrubStart}
        onpointerup={onScrubEnd}
      />
    </div>
  </div>
{/if}

<style>
  .log-player {
    position: absolute;
    top: 62px;
    left: 50%;
    transform: translateX(-50%);
    width: 800px;
    max-width: calc(100vw - 40px);
    z-index: 50;
    background: rgba(46, 46, 46, 0.92);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    padding: 8px 14px 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    user-select: none;
  }

  .log-player-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .log-player-source {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }

  .log-player-source-btn {
    background: #434343;
    border: 1px solid #555;
    color: #949494;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 3px;
    cursor: pointer;
    line-height: 1;
  }

  .log-player-source-btn.active {
    background: #37a8db;
    color: #fff;
    border-color: #339cc1;
  }

  .log-player-source-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .log-player-title {
    flex: 1;
    text-align: center;
    font-size: 13px;
    font-weight: 600;
    color: #e0e0e0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .log-player-firmware {
    font-weight: 400;
    color: #949494;
    font-size: 12px;
  }

  .log-player-close {
    background: none;
    border: none;
    color: #949494;
    font-size: 16px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
    flex-shrink: 0;
  }

  .log-player-close:hover {
    color: #d40000;
  }

  .log-player-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .log-player-time {
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 12px;
    color: #949494;
    min-width: 60px;
    text-align: center;
    flex-shrink: 0;
  }

  .log-player-buttons {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 2px;
    flex: 1;
  }

  .log-player-btn {
    background: #434343;
    border: 1px solid #555;
    color: #e0e0e0;
    font-size: 11px;
    padding: 4px 7px;
    border-radius: 3px;
    cursor: pointer;
    line-height: 1;
    transition: background 0.2s ease, border-color 0.2s ease;
  }

  .log-player-btn:hover {
    background: rgba(55, 168, 219, 0.15);
    border-color: #37a8db;
  }

  .log-player-btn.play-btn {
    font-size: 14px;
    padding: 4px 10px;
    background: #37a8db;
    color: #fff;
    border-color: #339cc1;
  }

  .log-player-btn.play-btn:hover {
    background: #45bce5;
  }

  .log-player-btn.speed-btn {
    font-weight: 700;
    min-width: 32px;
    text-align: center;
    color: #37a8db;
  }

  .log-player-scrubber {
    padding: 2px 0 0;
  }

  .log-player-slider {
    width: 100%;
    height: 6px;
    -webkit-appearance: none;
    appearance: none;
    background: #434343;
    border-radius: 3px;
    outline: none;
    cursor: pointer;
  }

  .log-player-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #37a8db;
    border: 2px solid #e0e0e0;
    cursor: pointer;
  }

  .log-player-slider::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #37a8db;
    border: 2px solid #e0e0e0;
    cursor: pointer;
  }
</style>