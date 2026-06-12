import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import path from "path";
import fs from "fs";
import { createRequire } from "module";
import { execSync } from "child_process";
import sirv from "sirv";

const require = createRequire(import.meta.url);

const host = process.env.TAURI_DEV_HOST;

// ── Build stamp (injected via `define`, surfaced in the About dialog) ──
// Short commit id (+ "-dirty" if the tree has uncommitted changes) so a bug report can be tied to an exact
// build — no manual version bump per build. Falls back to "unknown" when git isn't available (e.g. tarball).
const pkg = require("./package.json");
function gitCommit() {
  try {
    const hash = execSync("git rev-parse --short HEAD", { stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
    const dirty = execSync("git status --porcelain", { stdio: ["ignore", "pipe", "ignore"] }).toString().trim().length > 0;
    return hash + (dirty ? "-dirty" : "");
  } catch {
    return "unknown";
  }
}
const BUILD_STAMP = {
  __APP_VERSION__: JSON.stringify(pkg.version),
  __GIT_COMMIT__: JSON.stringify(gitCommit()),
  __BUILD_DATE__: JSON.stringify(new Date().toISOString().slice(0, 10)),
};

// Cesium asset source directory (use require.resolve to avoid URL-encoding issues)
const cesiumSource = path.join(
  path.dirname(require.resolve('cesium/package.json')), 'Build', 'Cesium'
);

/**
 * Custom Vite plugin: serves Cesium assets from node_modules in dev mode,
 * copies them to the build output for production.
 */
function cesiumPlugin() {
  return {
    name: 'cesium-assets',
    /** @param {import('vite').ViteDevServer} server */
    configureServer(server) {
      // Serve /cesium/* directly from the Cesium build directory during dev
      server.middlewares.use('/cesium', sirv(cesiumSource, { dev: true }));
    },
    /** @param {{ dir?: string }} options */
    async writeBundle(options) {
      // Copy Cesium assets to the build output directory for production
      const outDir = options.dir || 'build';
      const destDir = path.join(outDir, 'cesium');
      const dirs = ['Workers', 'ThirdParty', 'Assets', 'Widgets'];
      for (const dir of dirs) {
        const src = path.join(cesiumSource, dir);
        const dest = path.join(destDir, dir);
        if (fs.existsSync(src)) {
          fs.cpSync(src, dest, { recursive: true });
        }
      }
    },
  };
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    sveltekit(),
    cesiumPlugin(),
  ],

  define: {
    CESIUM_BASE_URL: JSON.stringify('/cesium'),
    ...BUILD_STAMP,
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
