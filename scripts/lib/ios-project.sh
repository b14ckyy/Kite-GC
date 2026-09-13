# ============================================================
# Shared iOS Xcode-project plumbing for scripts/build-ios.sh and
# scripts/run-ipad.sh. Sourced, not executed; the caller has already
# cd'd to the repository root.
# ============================================================

# src-tauri/gen/apple is generated and gitignored, so nothing in it can be
# committed: every fix to the generated project has to be re-applied here.
GEN_APPLE="src-tauri/gen/apple"

# `tauri ios init` lists Externals/ as a plain source group. XcodeGen has no default build phase for
# a `.a` and falls back to `resources` (SourceGenerator.getDefaultBuildPhase), so the Rust static
# library is added to "Copy Bundle Resources" as well; the link itself comes from the target's
# `dependencies: - framework: libapp.a`, not from the group. The copy puts the library inside the
# app bundle, which App Store validation rejects:
#
#   Invalid bundle structure. The "Kite Ground Control.app/libapp.a" binary file is not permitted.
#
# It also carries the library into the ipa for nothing (56 MB before, 19 MB after), and once several
# copies exist side by side (debug + release, device + simulator) Xcode fails outright with
# "Multiple commands produce .../libapp.a".
#
# Upstream fixes this in tauri-apps/tauri#15890 by adding `excludes: ["**/*.a"]` to the group, which
# is not in a released CLI yet (2.11.4 still ships the bare entry). This patch is compatible with
# that template: it only adds `buildPhase`, leaving any `excludes` in place.
IOS_YML_PATCHED=0

# Does the Externals source group already carry a buildPhase?
_ios_yml_has_build_phase() {
    awk '$0 ~ /^[[:space:]]*- path: Externals$/ {
             getline nxt
             if (nxt ~ /^[[:space:]]*buildPhase:/) found = 1
         }
         END { exit !found }' "$1"
}

ios_patch_externals_build_phase() {
    local yml="$GEN_APPLE/project.yml"
    if [ ! -f "$yml" ]; then
        echo "[ERROR] $yml not found. Run with FORCE_INIT=1 to regenerate the Xcode project."
        exit 1
    fi
    if _ios_yml_has_build_phase "$yml"; then
        echo "      Externals already excluded from Copy Bundle Resources."
        return 0
    fi
    awk '
        { print }
        /^[[:space:]]*- path: Externals$/ {
            match($0, /^[[:space:]]*/)
            print substr($0, 1, RLENGTH) "  buildPhase: none"
        }
    ' "$yml" > "$yml.tmp"
    # Never build on a silent miss: a template change in Tauri that renames or re-indents the group
    # would otherwise put libapp.a back in the bundle, and the rejection would only surface at
    # upload, after a signed archive and a wait.
    if ! _ios_yml_has_build_phase "$yml.tmp"; then
        rm -f "$yml.tmp"
        echo "[ERROR] Could not exclude Externals from Copy Bundle Resources in $yml."
        echo "        Add 'buildPhase: none' under the Externals source group by hand, or the app"
        echo "        will contain libapp.a and App Store validation will reject it."
        exit 1
    fi
    mv "$yml.tmp" "$yml"
    IOS_YML_PATCHED=1
    echo "      Excluded Externals from Copy Bundle Resources (keeps libapp.a out of the app)."
}

# Is the existing pbxproj one that still copies the library into the bundle? This is the state left
# by any earlier run: the project was generated while Externals/ already held a built libapp.a.
_ios_pbxproj_copies_libapp() {
    local pbx="$GEN_APPLE/kite-gc.xcodeproj/project.pbxproj"
    [ -f "$pbx" ] && grep -q 'libapp\.a in Resources' "$pbx"
}

# Only `tauri ios init` runs XcodeGen (crates/tauri-cli/src/mobile/ios/project.rs is its single call
# site). `tauri ios build` and `tauri ios dev` load the existing pbxproj and synchronize values into
# it, so a patched project.yml on its own reaches nothing: the project has to be regenerated for the
# change to take effect. Tauri writes the signing team and configuration back afterwards, so a
# regenerated project loses nothing that a re-init would not lose as well.
ios_regenerate_project() {
    if ! command -v xcodegen &> /dev/null; then
        echo "[ERROR] xcodegen not found, so the Xcode project cannot be regenerated."
        echo "        Install it with: brew install xcodegen"
        exit 1
    fi
    echo "      Regenerating the Xcode project from project.yml (xcodegen)..."
    xcodegen generate --spec "$GEN_APPLE/project.yml" --quiet
    if _ios_pbxproj_copies_libapp; then
        echo "[ERROR] The regenerated project still copies libapp.a into the bundle."
        exit 1
    fi
}

# Generate the Xcode project if it is missing (or FORCE_INIT=1), then make sure it is one that keeps
# libapp.a out of the app bundle. Regenerates only when something actually needs it, so a project
# that is already correct is left alone.
ios_prepare_project() {
    if [ ! -d "$GEN_APPLE" ] || [ "${FORCE_INIT:-0}" = "1" ]; then
        echo "      Generating the Xcode project (tauri ios init)..."
        npm run tauri ios init
    else
        echo "      $GEN_APPLE already present — skipping init (FORCE_INIT=1 to redo)."
    fi
    ios_patch_externals_build_phase
    if [ "$IOS_YML_PATCHED" = "1" ] || _ios_pbxproj_copies_libapp; then
        ios_regenerate_project
    else
        echo "      Xcode project already keeps libapp.a out of the bundle."
    fi
}
