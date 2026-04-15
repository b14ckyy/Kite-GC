<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { connection, availablePorts } from "$lib/stores/connection";
  import type { FcInfo, PortInfo, FeatureSet } from "$lib/stores/connection";
  import { settings } from "$lib/stores/settings";
  import type { AppSettings } from "$lib/stores/settings";
  import { telemetry, startTelemetryListeners, stopTelemetryListeners, resetTelemetry } from "$lib/stores/telemetry";
  import { get } from "svelte/store";
  import Map from "$lib/components/Map.svelte";

  let appVersion = $state("...");
  let selectedPort = $state("");
  let selectedBaud = $state(115200);
  let isConnecting = $state(false);
  let errorMsg = $state("");
  let navPanelOpen = $state(false);
  let activeTab = $state("uav-info");

  // Dev-only debug panel (tree-shaken in production builds)
  const DEV_MODE = import.meta.env.DEV;
  let debugOpen = $state(false);
  let DebugPanelCmp: any = $state(null);
  if (DEV_MODE) {
    import('$lib/components/DebugPanel.svelte').then(m => { DebugPanelCmp = m.default; });
  }

  // Reactive telemetry subscription
  let telem = $state(get(telemetry));
  telemetry.subscribe((t) => { telem = t; });

  // Settings state for the settings panel
  let attitudeRateHz = $state(5);
  let positionRateHz = $state(2);
  let airspeedEnabled = $state(false);

  // INAV arming flags (bit positions from INAV source)
  const ARMING_FLAG_ARMED = 2; // bit 2 = ARMED

  function isArmed(): boolean {
    return telem.lastUpdate > 0 && (telem.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
  }

  function getArmingLabel(): string {
    if (!telem.lastUpdate) return "";
    return isArmed() ? "ARMED" : "DISARMED";
  }

  function getGpsFixLabel(): string {
    if (!telem.lastUpdate || telem.fixType === 0) return "NO FIX";
    const types: Record<number, string> = { 1: "2D", 2: "3D", 3: "3D DGPS" };
    return types[telem.fixType] || `FIX:${telem.fixType}`;
  }

  // Platform type names from flyingPlatformType_e
  const platformTypes: Record<number, string> = {
    0: "Multirotor", 1: "Airplane", 2: "Helicopter", 3: "Tricopter",
    4: "Rover", 5: "Boat", 6: "Other",
  };

  const baudRates = [115200, 57600, 38400, 19200, 9600, 230400, 460800, 921600];

  const tabs = [
    { id: "uav-info", label: "UAV Info", icon: "✈" },
    { id: "settings", label: "Settings", icon: "⚙" },
    { id: "mission", label: "Mission", icon: "◎" },
  ];

  let ports: PortInfo[] = $state([]);
  let connStatus: string = $state("disconnected");
  let fcInfo: FcInfo | null = $state(null);

  // Subscribe to stores
  connection.subscribe((c) => {
    connStatus = c.status;
    fcInfo = c.fcInfo;
  });
  availablePorts.subscribe((p) => {
    ports = p;
  });

  // Restore persisted settings
  const saved = get(settings);
  selectedPort = saved.lastPort;
  selectedBaud = saved.lastBaud;
  navPanelOpen = saved.navPanelOpen;
  activeTab = saved.activeTab;
  attitudeRateHz = saved.attitudeRateHz;
  positionRateHz = saved.positionRateHz;
  airspeedEnabled = saved.airspeedEnabled;

  function toggleNavPanel() {
    navPanelOpen = !navPanelOpen;
    settings.patch({ navPanelOpen });
    // Let the map recalculate its size after panel animation
    setTimeout(() => window.dispatchEvent(new Event("resize")), 320);
  }

  function selectTab(tabId: string) {
    activeTab = tabId;
    settings.patch({ activeTab });
    if (!navPanelOpen) {
      navPanelOpen = true;
      settings.patch({ navPanelOpen: true });
      setTimeout(() => window.dispatchEvent(new Event("resize")), 320);
    }
  }

  async function loadInfo() {
    appVersion = await invoke("get_app_version");
    await refreshPorts();
  }

  async function refreshPorts() {
    try {
      const result = await invoke<PortInfo[]>("list_serial_ports");
      availablePorts.set(result);
      if (result.length > 0 && !selectedPort) {
        selectedPort = result[0].path;
      }
      if (selectedPort && result.some((p) => p.path === selectedPort)) {
        // port still valid
      } else if (result.length > 0) {
        selectedPort = result[0].path;
      }
    } catch (e) {
      console.error("Failed to list ports:", e);
    }
  }

  async function handleConnect() {
    if (connStatus === "connected") {
      try {
        stopTelemetryListeners();
        resetTelemetry();
        await invoke("disconnect");
        connection.set({
          status: "disconnected",
          port: "",
          baudRate: selectedBaud,
          errorMessage: "",
          fcInfo: null,
        });
        errorMsg = "";
      } catch (e: any) {
        errorMsg = e.toString();
      }
      return;
    }

    if (!selectedPort) {
      errorMsg = "No port selected";
      return;
    }

    isConnecting = true;
    errorMsg = "";
    connection.update((c) => ({ ...c, status: "connecting" }));

    settings.patch({ lastPort: selectedPort, lastBaud: selectedBaud });

    try {
      const info = await invoke<FcInfo>("connect", {
        port: selectedPort,
        baudRate: selectedBaud,
        attitudeRateHz: attitudeRateHz,
        positionRateHz: positionRateHz,
        airspeedEnabled: airspeedEnabled,
      });
      connection.set({
        status: "connected",
        port: selectedPort,
        baudRate: selectedBaud,
        errorMessage: "",
        fcInfo: info,
      });
      await startTelemetryListeners();
    } catch (e: any) {
      errorMsg = e.toString();
      connection.set({
        status: "error",
        port: "",
        baudRate: selectedBaud,
        errorMessage: e.toString(),
        fcInfo: null,
      });
    } finally {
      isConnecting = false;
    }
  }

  loadInfo();
</script>

<main class="app">
  <!-- ======= TOOLBAR ======= -->
  <header class="toolbar">
    <div class="toolbar-left">
      <div class="logo">INAV GCS</div>
      <span class="version">v{appVersion}</span>
    </div>
    <div class="toolbar-center">
      <div class="sensor-bar">
        <div class="sensor" class:active={telem.sensorGyro === 1} class:error={telem.sensorGyro === 3} title="Gyroscope">GYRO</div>
        <div class="sensor" class:active={telem.sensorAcc === 1} class:error={telem.sensorAcc === 3} title="Accelerometer">ACC</div>
        <div class="sensor" class:active={telem.sensorMag === 1} class:error={telem.sensorMag === 3} title="Magnetometer">MAG</div>
        <div class="sensor" class:active={telem.sensorBaro === 1} class:error={telem.sensorBaro === 3} title="Barometer">BARO</div>
        <div class="sensor" class:active={telem.sensorGps === 1 && telem.fixType >= 2} class:warning={telem.sensorGps === 1 && telem.fixType < 2} class:error={telem.sensorGps === 3} title="GPS: {getGpsFixLabel()} {telem.numSat}S">GPS</div>
      </div>
    </div>
    <div class="toolbar-right">
      <div class="port-controls">
        {#if connStatus !== "connected"}
          <select class="port-select" bind:value={selectedPort}>
            {#if ports.length === 0}
              <option value="">No ports found</option>
            {:else}
              {#each ports as port}
                <option value={port.path}>{port.label}</option>
              {/each}
            {/if}
          </select>
          <select class="baud-select" bind:value={selectedBaud}>
            {#each baudRates as baud}
              <option value={baud}>{baud}</option>
            {/each}
          </select>
          <button class="refresh-btn" onclick={refreshPorts} title="Refresh ports">⟳</button>
        {/if}
      </div>
      <button
        class="connect-btn"
        class:connected={connStatus === "connected"}
        class:connecting={isConnecting}
        onclick={handleConnect}
        disabled={isConnecting}
      >
        {#if isConnecting}
          Connecting...
        {:else if connStatus === "connected"}
          Disconnect
        {:else}
          Connect
        {/if}
      </button>
    </div>
  </header>

  <!-- ======= MAP (always fullscreen behind everything) ======= -->
  <div class="map-fullscreen">
    <Map />
  </div>

  <!-- ======= FLOATING NAV PANEL SYSTEM ======= -->
  <div class="nav-rail" class:open={navPanelOpen}>
    <!-- Hamburger button -->
    <button class="hamburger-btn" onclick={toggleNavPanel} title={navPanelOpen ? "Close panel" : "Open panel"}>
      <span class="hamburger-icon" class:open={navPanelOpen}>
        <span></span>
        <span></span>
        <span></span>
      </span>
    </button>

    <!-- Tab buttons (visible only when panel is open) -->
    {#if navPanelOpen}
      <div class="tab-buttons">
        {#each tabs as tab}
          <button
            class="tab-btn"
            class:active={activeTab === tab.id}
            onclick={() => selectTab(tab.id)}
            title={tab.label}
          >
            <span class="tab-icon">{tab.icon}</span>
            <span class="tab-label">{tab.label}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <!-- Floating panel content -->
  {#if navPanelOpen}
    <div class="nav-panel">
      <div class="panel-content">
        <!-- UAV Info Tab -->
        {#if activeTab === "uav-info"}
          {#if connStatus === "connected" && fcInfo}
            <section class="panel-section">
              <h4 class="section-heading">Flight Controller</h4>
              <div class="fc-info-grid">
                <span class="fc-label">Variant</span>
                <span class="fc-value">{fcInfo.fc_variant}</span>
                <span class="fc-label">Version</span>
                <span class="fc-value">{fcInfo.fc_version}</span>
                <span class="fc-label">Board</span>
                <span class="fc-value">{fcInfo.board_id}</span>
                <span class="fc-label">Type</span>
                <span class="fc-value">{platformTypes[fcInfo.platform_type] ?? `Unknown (${fcInfo.platform_type})`}</span>
                <span class="fc-label">API</span>
                <span class="fc-value">{fcInfo.api_version}</span>
                {#if fcInfo.hardware_revision > 0}
                  <span class="fc-label">HW Rev</span>
                  <span class="fc-value">{fcInfo.hardware_revision}</span>
                {/if}
              </div>
            </section>

            {#if fcInfo.features}
              <section class="panel-section">
                <h4 class="section-heading">Features</h4>
                <div class="feature-list">
                  <span class="feature-badge available">Telemetry</span>
                  <span class="feature-badge" class:available={fcInfo.features.autoland_config} class:unavailable={!fcInfo.features.autoland_config} title="INAV 7.1+">Autoland</span>
                  <span class="feature-badge" class:available={fcInfo.features.geozones} class:unavailable={!fcInfo.features.geozones} title="INAV 8.0+">Geozones</span>
                  <span class="feature-badge" class:available={fcInfo.features.msp_rc} class:unavailable={!fcInfo.features.msp_rc} title="INAV 8.0+">MSP-RC</span>
                  <span class="feature-badge" class:available={fcInfo.features.aux_rc} class:unavailable={!fcInfo.features.aux_rc} title="INAV 9.1+">AUX-RC</span>
                </div>
              </section>
            {/if}
          {:else}
            <div class="panel-empty">
              <span class="panel-empty-icon">⊘</span>
              <span>Not connected</span>
            </div>
          {/if}

        <!-- Settings Tab -->
        {:else if activeTab === "settings"}
          <section class="panel-section">
            <h4 class="section-heading">Telemetry Rates</h4>
            <div class="setting-row">
              <label class="setting-label" for="attitude-rate">Attitude</label>
              <select id="attitude-rate" class="setting-select" bind:value={attitudeRateHz} onchange={() => settings.patch({ attitudeRateHz })}>
                <option value={1}>1 Hz</option>
                <option value={2}>2 Hz</option>
                <option value={3}>3 Hz</option>
                <option value={5}>5 Hz</option>
              </select>
            </div>
            <div class="setting-row">
              <label class="setting-label" for="position-rate">GPS Position</label>
              <select id="position-rate" class="setting-select" bind:value={positionRateHz} onchange={() => settings.patch({ positionRateHz })}>
                <option value={1}>1 Hz</option>
                <option value={2}>2 Hz</option>
                <option value={3}>3 Hz</option>
                <option value={5}>5 Hz</option>
              </select>
            </div>
          </section>
          <section class="panel-section">
            <h4 class="section-heading">Optional Modules</h4>
            <div class="setting-row">
              <label class="setting-label" for="airspeed-toggle">Airspeed</label>
              <label class="toggle-switch">
                <input type="checkbox" id="airspeed-toggle" bind:checked={airspeedEnabled} onchange={() => settings.patch({ airspeedEnabled })} />
                <span class="toggle-slider"></span>
              </label>
            </div>
          </section>
          <section class="panel-section">
            <p class="setting-hint">Rate changes take effect on next connect.</p>
          </section>

        <!-- Mission Tab -->
        {:else if activeTab === "mission"}
          <div class="panel-empty">
            <span class="panel-empty-icon">◎</span>
            <span>Mission Control — coming soon</span>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  <!-- ======= BOTTOM TELEMETRY OVERLAY ======= -->
  <div class="telemetry-overlay">
    <div class="telemetry-strip">
      {#if telem.lastUpdate > 0 && isArmed()}
        <div class="telem-widget armed-widget">
          <span class="telem-label">STATUS</span>
          <span class="telem-value armed-text">ARMED</span>
        </div>
      {/if}
      <div class="telem-widget">
        <span class="telem-label">ALT</span>
        <span class="telem-value">{telem.lastUpdate ? `${telem.altitude.toFixed(1)} m` : '— m'}</span>
      </div>
      <div class="telem-widget">
        <span class="telem-label">SPD</span>
        <span class="telem-value">{telem.lastUpdate ? `${(telem.groundSpeed * 3.6).toFixed(0)} km/h` : '— km/h'}</span>
      </div>
      <div class="telem-widget">
        <span class="telem-label">VARIO</span>
        <span class="telem-value">{telem.lastUpdate ? `${telem.vario >= 0 ? '+' : ''}${telem.vario.toFixed(1)} m/s` : '— m/s'}</span>
      </div>
      <div class="telem-widget">
        <span class="telem-label">BAT</span>
        <span class="telem-value">{telem.lastUpdate ? `${telem.voltage.toFixed(1)}V ${telem.current.toFixed(1)}A` : '—V'}</span>
      </div>
      <div class="telem-widget">
        <span class="telem-label">SATS</span>
        <span class="telem-value">{telem.lastUpdate ? `${telem.numSat} ${getGpsFixLabel()}` : '—'}</span>
      </div>
    </div>
  </div>

  <!-- ======= DEBUG PANEL (dev only) ======= -->
  {#if DEV_MODE && debugOpen && DebugPanelCmp}
    <DebugPanelCmp onclose={() => debugOpen = false} />
  {/if}

  <!-- ======= ERROR BAR ======= -->
  {#if errorMsg}
    <div class="error-bar">
      <span>{errorMsg}</span>
      <button class="error-dismiss" onclick={() => (errorMsg = "")}>✕</button>
    </div>
  {/if}

  <!-- ======= STATUS BAR ======= -->
  <footer class="statusbar">
    <div class="statusbar-left">
      <span
        class="status-indicator"
        class:connected={connStatus === "connected"}
        class:disconnected={connStatus !== "connected"}
      ></span>
      <span>
        {#if connStatus === "connected" && fcInfo}
          {fcInfo.fc_variant} {fcInfo.fc_version} on {$connection.port}
        {:else if connStatus === "connecting"}
          Connecting...
        {:else}
          Disconnected
        {/if}
      </span>
      {#if DEV_MODE}
        <button class="debug-btn" class:open={debugOpen} onclick={() => debugOpen = !debugOpen} title="MSP Debug Monitor">
          🔧 Debug
        </button>
      {/if}
    </div>
    <div class="statusbar-right">
      {#if connStatus === "connected" && telem.lastUpdate > 0}
        <span class="status-arming" class:armed={isArmed()} class:disarmed={!isArmed()}>
          {getArmingLabel()}
        </span>
        <span class="status-sep">|</span>
      {/if}
      <span>INAV GCS</span>
    </div>
  </footer>
</main>

<style>
  /* ============================================================
     INAV GCS Theme — Floating Panel Layout
     Color palette derived from INAV Configurator
     https://github.com/iNavFlight/inav-configurator
     ============================================================ */

  :global(body) {
    margin: 0;
    padding: 0;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    background-color: #3d3f3e;
    color: #e0e0e0;
    overflow: hidden;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    position: relative;
  }

  /* --- Header / Toolbar --- */
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 16px;
    height: 50px;
    background: #2e2e2e;
    border-bottom: 3px solid #37a8db;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.3);
    position: relative;
    z-index: 200;
  }

  .toolbar-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .toolbar-center {
    display: flex;
    align-items: center;
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .logo {
    font-size: 20px;
    font-weight: 700;
    color: #37a8db;
    letter-spacing: 0.5px;
  }

  .version {
    font-size: 11px;
    color: #949494;
  }

  /* --- Sensor Status Bar --- */
  .sensor-bar {
    display: flex;
    gap: 1px;
    background: #434343;
    border-radius: 5px;
    border: 1px solid #272727;
    box-shadow: 0 2px 0 rgba(92, 92, 92, 0.5);
    overflow: hidden;
  }

  .sensor {
    padding: 6px 12px;
    font-size: 10px;
    font-weight: 600;
    color: #4f4f4f;
    text-shadow: 0 1px rgba(0, 0, 0, 1.0);
    background: #434343 linear-gradient(to bottom, transparent, rgba(0, 0, 0, 0.45));
    border-right: 1px solid #373737;
    text-align: center;
    min-width: 36px;
  }

  .sensor:last-child {
    border-right: none;
  }

  /* --- Port Controls --- */
  .port-controls {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .port-select,
  .baud-select {
    padding: 4px 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 12px;
  }

  .port-select {
    min-width: 160px;
  }

  .baud-select {
    min-width: 80px;
  }

  .refresh-btn {
    padding: 4px 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 14px;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .refresh-btn:hover {
    background: #555;
  }

  /* --- Connect Button --- */
  .connect-btn {
    padding: 6px 16px;
    background: #37a8db;
    border: 1px solid #339cc1;
    border-radius: 3px;
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    text-shadow: 0 1px rgba(0, 0, 0, 0.25);
    cursor: pointer;
    transition: background-color 0.2s ease;
    min-width: 90px;
  }

  .connect-btn:hover:not(:disabled) {
    background: #45bce5;
  }

  .connect-btn:disabled {
    opacity: 0.7;
    cursor: wait;
  }

  .connect-btn.connected {
    background: #e60000;
    border-color: #fe0000;
  }

  .connect-btn.connected:hover {
    background: #f21212;
  }

  .connect-btn.connecting {
    background: #f5a623;
    border-color: #e09a1e;
  }

  /* --- Full-screen Map --- */
  .map-fullscreen {
    position: absolute;
    top: 53px; /* toolbar height + border */
    left: 0;
    right: 0;
    bottom: 24px; /* statusbar height */
    z-index: 0;
  }

  /* --- Floating Navigation Rail (hamburger + tab buttons) --- */
  .nav-rail {
    position: absolute;
    top: 65px;
    left: 12px;
    display: flex;
    flex-direction: column;
    gap: 0;
    z-index: 100;
    transition: left 0.3s ease;
  }

  .nav-rail.open {
    left: 12px;
  }

  /* --- Hamburger Button --- */
  .hamburger-btn {
    width: 42px;
    height: 42px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background-color 0.2s;
    backdrop-filter: blur(8px);
  }

  .hamburger-btn:hover {
    background: rgba(55, 168, 219, 0.25);
  }

  .hamburger-icon {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 20px;
    transition: transform 0.3s ease;
  }

  .hamburger-icon span {
    display: block;
    height: 2px;
    background: #37a8db;
    border-radius: 1px;
    transition: transform 0.3s ease, opacity 0.2s ease;
  }

  .hamburger-icon.open span:nth-child(1) {
    transform: translateY(6px) rotate(45deg);
  }

  .hamburger-icon.open span:nth-child(2) {
    opacity: 0;
  }

  .hamburger-icon.open span:nth-child(3) {
    transform: translateY(-6px) rotate(-45deg);
  }

  /* --- Tab Buttons --- */
  .tab-buttons {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 4px;
  }

  .tab-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 42px;
    height: 38px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 6px;
    color: #949494;
    font-size: 11px;
    cursor: pointer;
    padding: 0;
    justify-content: center;
    overflow: hidden;
    transition: width 0.3s ease, background-color 0.2s;
    backdrop-filter: blur(8px);
    white-space: nowrap;
  }

  .tab-btn:hover {
    background: rgba(55, 168, 219, 0.15);
    color: #e0e0e0;
  }

  .tab-btn.active {
    background: rgba(55, 168, 219, 0.25);
    border-color: #37a8db;
    color: #37a8db;
  }

  .tab-icon {
    font-size: 16px;
    flex-shrink: 0;
  }

  .tab-label {
    display: none;
  }

  /* --- Floating Nav Panel --- */
  .nav-panel {
    position: absolute;
    top: 65px;
    left: 62px; /* after the rail buttons */
    width: 240px;
    max-height: calc(100vh - 53px - 24px - 80px); /* toolbar - statusbar - margins */
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    z-index: 90;
    overflow-y: auto;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(12px);
    animation: panel-slide-in 0.25s ease-out;
  }

  @keyframes panel-slide-in {
    from {
      opacity: 0;
      transform: translateX(-16px);
    }
    to {
      opacity: 1;
      transform: translateX(0);
    }
  }

  .panel-content {
    padding: 14px;
  }

  .panel-section {
    margin-bottom: 16px;
  }

  .section-heading {
    margin: 0 0 8px 0;
    font-size: 11px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 32px 0;
    color: #555;
    font-size: 12px;
  }

  .panel-empty-icon {
    font-size: 28px;
    opacity: 0.4;
  }

  .fc-info-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    font-size: 12px;
  }

  .fc-label {
    color: #949494;
  }

  .fc-value {
    color: #e0e0e0;
    font-weight: 600;
  }

  .feature-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .feature-badge {
    padding: 3px 8px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 600;
  }

  .feature-badge.available {
    background: rgba(89, 170, 41, 0.2);
    color: #59aa29;
    border: 1px solid rgba(89, 170, 41, 0.4);
  }

  .feature-badge.unavailable {
    background: rgba(80, 80, 80, 0.2);
    color: #555;
    border: 1px solid #444;
    text-decoration: line-through;
  }

  /* --- Bottom Telemetry Overlay --- */
  .telemetry-overlay {
    position: absolute;
    bottom: 30px; /* above statusbar */
    left: 50%;
    transform: translateX(-50%);
    z-index: 100;
    pointer-events: auto;
  }

  .telemetry-strip {
    display: flex;
    gap: 2px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 8px;
    padding: 4px;
    backdrop-filter: blur(12px);
    box-shadow: 0 -2px 12px rgba(0, 0, 0, 0.3);
  }

  .telem-widget {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 6px 14px;
    min-width: 60px;
    border-radius: 5px;
    background: rgba(0, 0, 0, 0.2);
  }

  .telem-label {
    font-size: 9px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .telem-value {
    font-size: 14px;
    font-weight: 700;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
  }

  /* --- Error Bar --- */
  .error-bar {
    position: absolute;
    bottom: 24px;
    left: 0;
    right: 0;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 12px;
    background: #d40000;
    color: #fff;
    font-size: 12px;
    z-index: 300;
  }

  .error-dismiss {
    background: none;
    border: none;
    color: #fff;
    font-size: 14px;
    cursor: pointer;
    padding: 0 4px;
  }

  /* --- Status Bar --- */
  .statusbar {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 10px;
    height: 24px;
    background: #2e2e2e;
    border-top: 1px solid #272727;
    font-size: 11px;
    color: #949494;
    z-index: 200;
  }

  .statusbar-left {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .status-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .status-indicator.disconnected {
    background: #d40000;
    box-shadow: 0 0 4px rgba(212, 0, 0, 0.5);
  }

  .status-indicator.connected {
    background: #59aa29;
    box-shadow: 0 0 4px rgba(89, 170, 41, 0.5);
  }

  /* --- Sensor bar active state --- */
  .sensor.active {
    color: #59aa29;
    text-shadow: 0 0 4px rgba(89, 170, 41, 0.3);
  }

  .sensor.warning {
    color: #f5a623;
    text-shadow: 0 0 4px rgba(245, 166, 35, 0.3);
  }

  .sensor.error {
    color: #d40000;
    text-shadow: 0 0 4px rgba(212, 0, 0, 0.3);
  }

  /* --- Armed widget in telemetry strip --- */
  .armed-widget {
    background: rgba(212, 0, 0, 0.25) !important;
    border: 1px solid rgba(212, 0, 0, 0.5);
  }

  .armed-text {
    color: #ff4444 !important;
    animation: armed-pulse 1.5s ease-in-out infinite;
  }

  @keyframes armed-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
  }

  /* --- Arming status in status bar --- */
  .statusbar-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .status-arming {
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.5px;
  }

  .status-arming.armed {
    color: #ff4444;
  }

  .status-arming.disarmed {
    color: #59aa29;
  }

  .status-sep {
    color: #555;
  }

  /* --- Settings panel controls --- */
  .setting-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 0;
  }

  .setting-label {
    font-size: 12px;
    color: #e0e0e0;
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

  .setting-hint {
    font-size: 10px;
    color: #666;
    margin: 4px 0 0 0;
    font-style: italic;
  }

  /* --- Toggle switch --- */
  .toggle-switch {
    position: relative;
    display: inline-block;
    width: 36px;
    height: 20px;
  }

  .toggle-switch input {
    opacity: 0;
    width: 0;
    height: 0;
  }

  .toggle-slider {
    position: absolute;
    cursor: pointer;
    top: 0; left: 0; right: 0; bottom: 0;
    background-color: #434343;
    border: 1px solid #555;
    border-radius: 20px;
    transition: background-color 0.2s;
  }

  .toggle-slider::before {
    content: "";
    position: absolute;
    height: 14px;
    width: 14px;
    left: 2px;
    bottom: 2px;
    background-color: #949494;
    border-radius: 50%;
    transition: transform 0.2s, background-color 0.2s;
  }

  .toggle-switch input:checked + .toggle-slider {
    background-color: rgba(55, 168, 219, 0.3);
    border-color: #37a8db;
  }

  .toggle-switch input:checked + .toggle-slider::before {
    transform: translateX(16px);
    background-color: #37a8db;
  }

  /* --- Dev Debug Button (status bar) --- */
  .debug-btn {
    background: none;
    border: 1px solid transparent;
    color: #666;
    font-size: 11px;
    cursor: pointer;
    padding: 0 6px;
    border-radius: 3px;
    margin-left: 8px;
    transition: color 0.2s, border-color 0.2s, background-color 0.2s;
  }

  .debug-btn:hover {
    color: #f5a623;
    border-color: rgba(245, 166, 35, 0.3);
  }

  .debug-btn.open {
    color: #f5a623;
    border-color: rgba(245, 166, 35, 0.4);
    background: rgba(245, 166, 35, 0.1);
  }
</style>
