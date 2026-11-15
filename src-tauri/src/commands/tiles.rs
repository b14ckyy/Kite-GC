// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Map tiles through reqwest (HTTP/2, pooled) instead of the WebView, whose per-host cap (and request
//! serialisation on WebKitGTK) makes tiles crawl in; a per-layer gate keeps us polite to community servers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;

/// Concurrent tile fetches **per layer** (keyed by URL template): bypassing the WebView drops the
/// browser's per-host limit, and community servers (OSM, OpenTopoMap) risk a ban beyond this.
const MAX_CONCURRENT_PER_LAYER: usize = 12;

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

/// One semaphore per layer key, kept for the app lifetime (there are only a handful of layers).
fn layer_gate(layer: &str) -> Arc<Semaphore> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    let mut map = GATES.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    map.entry(layer.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_PER_LAYER)))
        .clone()
}

/// Fetch one map tile; `layer` bounds concurrency. A Tauri `Response`, so the bytes travel as a
/// binary payload (an `ArrayBuffer`), not a JSON number array.
#[tauri::command]
pub async fn fetch_tile(url: String, layer: String) -> Result<tauri::ipc::Response, String> {
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
