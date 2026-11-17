<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Shown while the flight database is upgraded. No button and no way to dismiss it: closing Kite during
     the backup or the migration is what it is there to prevent. -->
<script lang="ts">
  let { title, message }: { title: string; message: string } = $props();
</script>

<div class="upg-backdrop" role="alertdialog" aria-modal="true" aria-label={title}>
  <div class="upg-box">
    <div class="upg-head">
      <span class="upg-spinner" aria-hidden="true"></span>
      <span class="upg-title">{title}</span>
    </div>
    <div class="upg-message">{message}</div>
  </div>
</div>

<style>
  .upg-backdrop {
    position: fixed; inset: 0; z-index: 10000;
    display: flex; align-items: center; justify-content: center;
    background: rgba(0, 0, 0, 0.6);
  }
  .upg-box {
    width: min(460px, calc(100vw - 32px));
    padding: 18px 20px;
    background: #2e2e2e; border: 1px solid #272727; border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
    font-family: 'Segoe UI', Tahoma, sans-serif; color: #e0e0e0;
  }
  .upg-head { display: flex; align-items: center; gap: 10px; margin-bottom: 10px; }
  .upg-title { font-size: 15px; font-weight: 600; color: #37a8db; }
  .upg-message { font-size: 13px; line-height: 1.5; color: #c8c8c8; white-space: pre-line; }
  .upg-spinner {
    width: 16px; height: 16px; flex: none;
    border: 2px solid rgba(55, 168, 219, 0.3); border-top-color: #37a8db; border-radius: 50%;
    animation: upg-spin 0.9s linear infinite;
  }
  @keyframes upg-spin { to { transform: rotate(360deg); } }
</style>
