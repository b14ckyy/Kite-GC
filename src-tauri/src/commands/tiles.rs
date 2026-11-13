// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Backend map-tile loader.
//!
//! Fetches map tiles through the Rust HTTP stack (reqwest → HTTP/2 + connection
//! pooling) instead of the OS WebView's HTTP client, for both the 2D (Leaflet)
//! and 3D (Cesium) tile pipelines. The WebView caps connections per host and, on
//! WebKitGTK/Linux, serialises requests to a single-host provider (e.g. ESRI),
//! which makes tiles crawl in; reqwest multiplexes over HTTP/2 without that cap,
//! so tile loading is fast and consistent across all platforms. A concurrency
//! gate keeps us a polite client toward community providers (see below).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;

/// Cap on concurrent tile fetches **per layer**. Bypassing the WebView removes the
/// browser's natural per-host connection limit, so a single pan could otherwise
/// fire 100+ requests at once — fine for CDN providers (ESRI/Carto) but a
/// policy/ban risk for community servers (OSM, OpenTopoMap). Capping *per layer*
/// (keyed by the layer's URL template) keeps every rate-sensitive provider — which
/// are all single-layer — at 12, while a multi-layer CDN style (ESRI Hybrid = base
/// + 2 reference overlays) gets 12 per layer and loads as fast as a single-layer
/// map. Easily tuned.
const MAX_CONCURRENT_PER_LAYER: usize = 12;

/// Shared HTTP client — pooled connections + HTTP/2, reused across all tile fetches.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(crate::user_agent::user_agent("tiles"))
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build tile HTTP client")
    })
}

/// Per-layer concurrency gate. One semaphore per layer key (created on first use,
/// kept for the app lifetime — there are only a handful of layers).
fn layer_gate(layer: &str) -> Arc<Semaphore> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    let mut map = GATES.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    map.entry(layer.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_PER_LAYER)))
        .clone()
}

/// Fetch a single map tile and return its raw bytes to the frontend. `layer` is a
/// stable per-layer key (the URL template) used to bound concurrency per layer.
///
/// Returned as a Tauri `Response` so the bytes travel over IPC as a real binary
/// payload (an `ArrayBuffer` on the JS side), not a JSON number array.
#[tauri::command]
pub async fn fetch_tile(url: String, layer: String) -> Result<tauri::ipc::Response, String> {
    // Hold a permit for the whole request so no more than MAX_CONCURRENT_PER_LAYER
    // of this layer are ever in flight; excess fetches queue and proceed as permits free up.
    let _permit = layer_gate(&layer)
        .acquire_owned()
        .await
        .map_err(|e| format!("tile gate closed: {e}"))?;
    let resp = client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("tile request failed: {}", describe_error(e)))?;
    if !resp.status().is_success() {
        return Err(format!("tile HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("tile body read failed: {}", describe_error(e)))?;
    Ok(tauri::ipc::Response::new(bytes.to_vec()))
}

/// Error text for the frontend: the error and its causes (DNS, connect, TLS …), WITHOUT the URL. A
/// user map's URL may carry an API key, and this text reaches the UI (Settings → Test); reqwest's own
/// message would quote the full URL.
fn describe_error(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    // A refused connection on the loopback (port 1 is practically never listened on) yields a real
    // reqwest error that carries the request URL; the described text must not. No proxy (an
    // environment proxy would answer instead) and a short timeout (a dropping firewall must not hang).
    #[tokio::test]
    async fn describe_error_drops_the_url() {
        let url = "http://127.0.0.1:1/tiles/1/0/0.png?key=SECRET123";
        let err = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("client")
            .get(url)
            .send()
            .await
            .expect_err("nothing listens on port 1");
        assert!(err.to_string().contains("SECRET123"), "precondition: reqwest quotes the URL");
        let text = describe_error(err);
        assert!(!text.contains("SECRET123"), "key leaked: {text}");
        assert!(!text.contains("127.0.0.1:1/tiles"), "URL leaked: {text}");
        assert!(!text.is_empty());
    }
}
