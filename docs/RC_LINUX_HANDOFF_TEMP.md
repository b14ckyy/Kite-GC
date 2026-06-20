# TEMP — RC HID Linux backend handoff (AI-only, delete after Linux verification)

Audience: an AI assistant on the Linux build machine. Purpose: diagnose/fix the Linux HID backend if it
fails to compile or detect a joystick. The Windows backend is verified working; Linux was written on a
Windows host and **never compiled there** (cfg-gated out). Full design: `docs/active/RC_CONTROL.md` §6.

## What this subsystem is
Native HID/joystick input for GCS RC control (INAV RC over MSP), Phase 1 = raw input only. A dedicated
Rust thread polls the selected device ~50 Hz and emits Tauri events to the Svelte frontend. NO channel
mapping / NO MSP yet — only raw axes/buttons/hats for a calibration view.

## Files
- `src-tauri/src/hid/mod.rs` — shared types + `HidManager` + thread loop + backend dispatch.
- `src-tauri/src/hid/linux.rs` — **the file to debug** (`#[cfg(target_os = "linux")]`).
- `src-tauri/src/hid/windows.rs` — Windows WGI backend (reference for expected behaviour; verified OK).
- `src-tauri/src/commands/hid.rs` — `hid_start` / `hid_stop` / `hid_select_device` Tauri commands.
- `src/lib/stores/hid.ts` + `src/lib/components/control/RcControlPanel.svelte` — frontend (unchanged by OS).

## Contract (must hold; identical to the Windows backend)
`mod.rs` defines:
```rust
trait HidBackend {
    fn poll(&mut self) -> Vec<HidDevice>;          // re-scan (self-throttled) + return current list
    fn snapshot(&mut self, id: usize) -> Option<HidSnapshot>;   // live raw state of one device
}
```
Shapes (serde → frontend, do not rename fields):
- `HidDevice { id: usize, name: String, uuid: String, axes: usize, buttons: usize, hats: usize }`
- `HidAxis { code: u32, value: f32 }`  — value normalised **−1.0..+1.0**, centre 0.
- `HidButton { code: u32, pressed: bool, value: f32 }` — value 0.0/1.0.
- `HidHat { code: u32, x: i32, y: i32 }` — x,y ∈ {−1,0,1}, **+y = up**.
- `HidSnapshot { id: usize, axes: Vec<HidAxis>, buttons: Vec<HidButton>, hats: Vec<HidHat> }`
`make_backend()` returns `Box::new(linux::EvdevBackend::new())` under cfg(linux).

## Dependency
`Cargo.toml` (Linux target): `evdev = "0.13"` (pure Rust, **not** `evdev-rs`/libevdev). Resolved 0.13.2.

## evdev 0.13 API the code relies on (verify these names if compile fails — they drifted in older versions)
- `evdev::enumerate() -> impl Iterator<Item = (std::path::PathBuf, evdev::Device)>` (opens each device).
- Code types: `evdev::AbsoluteAxisCode(pub u16)`, `evdev::KeyCode(pub u16)`.
- `Device::supported_absolute_axes() -> Option<&AttributeSetRef<AbsoluteAxisCode>>`
- `Device::supported_keys() -> Option<&AttributeSetRef<KeyCode>>`
- `AttributeSetRef::iter()` yields **owned** code values (`AbsoluteAxisCode` / `KeyCode`); `.0` is the u16.
- `AttributeSetRef::contains(T)` / `AttributeSet::contains(T)` take the code **by value**.
- `Device::get_abs_state() -> io::Result<[input_absinfo; AbsoluteAxisCode::COUNT]>` — index by `code as usize`.
- `Device::get_key_state() -> io::Result<AttributeSet<KeyCode>>`.
- `input_absinfo` is libc's struct; public i32 fields `value`, `minimum`, `maximum` are read by **field
  access only** (the type is deliberately NOT imported/named — do not add a `libc` dependency to name it).
- `Device::input_id() -> InputId` with `.vendor()/.product()/.version() -> u16`.
- `Device::unique_name()/physical_path()/name() -> Option<&str>`.
- `get_abs_state`/`get_key_state` are `&self` (no `&mut`); they ioctl current state — NO `fetch_events`
  draining is needed for a state view.

Older-version pitfalls if names don't resolve: `AbsoluteAxisType`→`AbsoluteAxisCode`, `Key`→`KeyCode`,
state getters returning different container types. Adjust to whatever 0.13.x actually exposes; keep the
`HidBackend` contract + output shapes unchanged.

## EvdevBackend behaviour (what linux.rs implements)
- `EvdevBackend { devices: Vec<DeviceEntry>, ids: HashMap<String,usize>, next_id, last_scan }`.
- `poll()`: rescan if `last_scan` older than 1000 ms (or never), then map `devices` → `Vec<HidDevice>`.
- `rescan()`: for each enumerated device passing `is_joystick`, build a `DeviceEntry`:
  - **is_joystick** = has `supported_absolute_axes()` AND some supported key in `0x120..0x140`
    (BTN_JOYSTICK..BTN_GAMEPAD) — excludes mice/touchpads/tablets.
  - **axis_codes** = supported ABS codes EXCLUDING the HAT range `0x10..=0x17`, sorted.
  - **hat_pairs** = `(0x10,0x11),(0x12,0x13),(0x14,0x15),(0x16,0x17)` where either code is supported.
  - **button_codes** = supported keys with code `>= 0x100` (BTN_*), sorted.
  - **uuid** = `unique_name()` (non-empty) else `physical_path()` else `"{vendor:04x}:{product:04x}:{version:04x}"`.
    Stable `id` assigned per uuid via the `ids` map (kept across rescans/reconnects).
  - entries sorted by `id`.
- `snapshot(id)`: `get_abs_state()` + `get_key_state()` once, then:
  - axis: `value = norm(info.value, info.minimum, info.maximum)`, `norm = clamp(2*(v-min)/(max-min)-1, -1,1)`.
  - hat: `x = signum(abs[xcode].value)`, `y = -signum(abs[ycode].value)` (evdev HAT Y negative = up → flip).
  - button: `pressed = key_state.contains(KeyCode(code))`, value 1.0/0.0.

## Runtime requirements / likely failure causes (not compile)
- **Permissions:** reading `/dev/input/event*` needs the user in group `input` (or udev rule). If
  `enumerate()` yields nothing or devices are skipped, this is the usual cause. `evdev::enumerate()`
  silently skips devices it cannot open.
- Some sticks expose two event nodes; the joystick filter should pick the one with ABS axes + BTN keys.

## How to verify
1. `cd src-tauri && cargo check` (Linux) → fix any evdev API-name mismatches per the pitfalls above.
2. `cargo test --no-run`, then `npm run check` (frontend, OS-independent — should already pass).
3. Run the app, Settings → Data → enable "RC Control", open the RC nav-rail tab, plug in a joystick:
   axes move, buttons light with sequential numbers, hats show an 8-way dot. Compare to Windows behaviour.

Delete this file once Linux is confirmed working.
