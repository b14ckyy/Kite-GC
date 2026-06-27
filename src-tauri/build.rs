use std::process::Command;

fn main() {
    // Git short hash → baked in as KITE_GIT_HASH for the log session header / About (best-effort:
    // "unknown" when git isn't available, e.g. a source tarball build). Rebuild when HEAD moves.
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=KITE_GIT_HASH={git_hash}");
    println!("cargo:rerun-if-changed=../.git/HEAD");

    tauri_build::build()
}
