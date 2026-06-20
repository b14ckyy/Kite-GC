// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC input methods — the reusable, pure "helper" layer that turns HID inputs into a channel value
// (docs/active/RC_CONTROL.md §7). Each RC channel is driven by one method. Values are NORMALISED
// −1..+1 internally (input/output stay comparable, trivial µs conversion); the UI shows the µs preview.
// Inputs are referenced by stable labels: A<n> = axis, B<n> = button, H<n> = hat direction (hats are
// treated like buttons). Stateful methods (adjust integrators, toggle/step) keep their state in
// `MethodState`; the step fn is pure (state in → state out) so this whole file is testable and later
// portable to Rust for the live MSP stream.

/** Normalised channel value (−1..+1) → RC µs (1000..2000). */
export function toUs(value: number): number {
  return Math.round(1500 + clamp(value, -1, 1) * 500);
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/** Abstract input frame — decouples the methods from the HID backend (built in the engine). */
export interface InputFrame {
  /** Axis value −1..+1 for label "A<n>" (0 if missing). */
  axis(label: string): number;
  /** Button pressed for label "B<n>" / "H<n>" (false if missing). */
  button(label: string): boolean;
}

// ── Method configs (discriminated union by `kind`) ───────────────────────────────────────────────

/** 1 axis → channel, directly. */
export interface PassthroughConfig {
  kind: 'passthrough';
  input: string;
  invert: boolean;
  /** Centre deadband 0..1 (≈0.001–0.1). */
  deadband: number;
}

/** 1 axis → rate of change (the axis sets how fast the channel moves). General (throttle, gimbal…). */
export interface AnalogAdjustConfig {
  kind: 'analogAdjust';
  input: string;
  invert: boolean;
  /** Full-deflection rate in normalised units/sec (1.0 ≈ 500 µs/s; 2.0 = full travel/s). */
  rate: number;
  deadband: number;
}

/** 2 axes/triggers → one adds, one subtracts; both at −1 (released) = centre. */
export interface DualAxisConfig {
  kind: 'dualAxis';
  inputAdd: string;
  inputSub: string;
  mode: 'absolute' | 'adjust';
  /** Rate for adjust mode (normalised units/sec). */
  rate: number;
}

/** 1 button momentary: high while pressed, low when released. */
export interface HoldConfig {
  kind: 'hold';
  input: string;
  low: number; // normalised
  high: number;
}

/** 1 button cycles sequentially through 2..6 positions (wrap). */
export interface ToggleConfig {
  kind: 'toggle';
  input: string;
  positions: number[]; // normalised values, length 2..6
  /** Hold time in ms required to advance (must hold, not tap); 0/undefined = instant. Anti-accidental
   *  for critical switches (e.g. a 2-position arming toggle on a gamepad). */
  holdMs?: number;
}

/** 2 buttons step a channel +/− through 3..15 discrete steps (clamp at ends). Capped at 15 so it fits
 *  a 4-bit AUX_RC channel (value 0 = "no update", leaving 15 usable levels). */
export interface ButtonStepConfig {
  kind: 'buttonStep';
  inputUp: string;
  inputDown: string;
  steps: number; // 3..15
}

/** 2 buttons ramp a channel +/− at a constant rate while held (clamp at ends). */
export interface ButtonAdjustConfig {
  kind: 'buttonAdjust';
  inputUp: string;
  inputDown: string;
  rate: number; // normalised units/sec
}

/** Every channel config also carries an optional display name (shown in the channel monitor). */
export type RcMethod = { name?: string } & (
  | PassthroughConfig
  | AnalogAdjustConfig
  | DualAxisConfig
  | HoldConfig
  | ToggleConfig
  | ButtonStepConfig
  | ButtonAdjustConfig
);

/** RC channel number (1..32) → its method config. */
export type RcChannelMap = Record<number, RcMethod>;

export type RcMethodKind = RcMethod['kind'];

// ── Runtime state (per channel) ──────────────────────────────────────────────────────────────────

export interface MethodState {
  /** Held output (−1..+1) for integrators / latched methods. */
  value: number;
  /** Position index for toggle / step. */
  pos: number;
  /** Previous pressed state per input label (button edge detection). */
  prev: Record<string, boolean>;
  /** Accumulated ms the toggle button has been held (for hold-to-toggle). */
  held: number;
}

/** Initial state for a method — every adjustable/value-holding method starts at the LOWEST µs value
 *  (−1 → 1000 µs), regardless of invert. (Passthrough is stateless: its value follows the input.) */
export function initState(method: RcMethod): MethodState {
  let value = -1;
  let pos = 0;
  if (method.kind === 'toggle' && method.positions.length) {
    // Start at the lowest position, whatever order the array is in (toggle output = positions[pos]).
    pos = method.positions.indexOf(Math.min(...method.positions));
    if (pos < 0) pos = 0;
    value = method.positions[pos];
  }
  return { value, pos, prev: {}, held: 0 };
}

// ── Helpers ────────────────────────────────────────────────────────────────────────────────────

/** Apply a centre deadband to a −1..+1 value (rescaled so it still reaches ±1). */
function deadband(v: number, db: number): number {
  if (db <= 0) return v;
  if (Math.abs(v) <= db) return 0;
  return Math.sign(v) * (Math.abs(v) - db) / (1 - db);
}

/** −1..+1 value for step `i` of `n` discrete steps. */
function stepValue(i: number, n: number): number {
  if (n <= 1) return -1;
  return -1 + (clamp(i, 0, n - 1) / (n - 1)) * 2;
}

/** Evenly-spaced normalised values for `count` positions (−1..+1). */
export function evenPositions(count: number): number[] {
  if (count <= 1) return [0];
  return Array.from({ length: count }, (_, i) => -1 + (i / (count - 1)) * 2);
}

/** Rising edge: pressed now, not before. Records the new state into `prev`. */
function edge(state: MethodState, label: string, pressed: boolean): boolean {
  const was = state.prev[label] ?? false;
  state.prev[label] = pressed;
  return pressed && !was;
}

// ── Step (pure: returns the channel value + the next state) ──────────────────────────────────────

export function stepMethod(
  method: RcMethod,
  state: MethodState,
  frame: InputFrame,
  dtMs: number,
): { value: number; state: MethodState } {
  const dt = dtMs / 1000;
  const next: MethodState = { value: state.value, pos: state.pos, prev: { ...state.prev }, held: state.held };

  switch (method.kind) {
    case 'passthrough': {
      let v = frame.axis(method.input);
      if (method.invert) v = -v;
      return { value: clamp(deadband(v, method.deadband), -1, 1), state: next };
    }
    case 'analogAdjust': {
      let a = frame.axis(method.input);
      if (method.invert) a = -a;
      a = deadband(a, method.deadband);
      next.value = clamp(state.value + a * method.rate * dt, -1, 1);
      return { value: next.value, state: next };
    }
    case 'dualAxis': {
      const add = (clamp(frame.axis(method.inputAdd), -1, 1) + 1) / 2; // 0..1 (released = 0)
      const sub = (clamp(frame.axis(method.inputSub), -1, 1) + 1) / 2;
      if (method.mode === 'absolute') {
        return { value: clamp(add - sub, -1, 1), state: next };
      }
      next.value = clamp(state.value + (add - sub) * method.rate * dt, -1, 1);
      return { value: next.value, state: next };
    }
    case 'hold': {
      return { value: frame.button(method.input) ? method.high : method.low, state: next };
    }
    case 'toggle': {
      const n = method.positions.length || 1;
      const pressed = frame.button(method.input);
      const hold = method.holdMs ?? 0;
      if (hold > 0) {
        // Hold-to-toggle: advance once when the press crosses the hold threshold; release to re-arm.
        if (pressed) {
          const before = state.held;
          next.held = before + dtMs;
          if (before < hold && next.held >= hold) next.pos = (state.pos + 1) % n;
        } else {
          next.held = 0;
        }
        next.prev[method.input] = pressed;
      } else if (edge(next, method.input, pressed)) {
        next.pos = (state.pos + 1) % n;
      }
      return { value: method.positions[next.pos] ?? -1, state: next };
    }
    case 'buttonStep': {
      const up = edge(next, method.inputUp, frame.button(method.inputUp));
      const down = edge(next, method.inputDown, frame.button(method.inputDown));
      if (up) next.pos = clamp(state.pos + 1, 0, method.steps - 1);
      if (down) next.pos = clamp(state.pos - 1, 0, method.steps - 1);
      next.value = stepValue(next.pos, method.steps);
      return { value: next.value, state: next };
    }
    case 'buttonAdjust': {
      const dir = (frame.button(method.inputUp) ? 1 : 0) - (frame.button(method.inputDown) ? 1 : 0);
      next.value = clamp(state.value + dir * method.rate * dt, -1, 1);
      return { value: next.value, state: next };
    }
  }
}

// ── Default factories (used by the config UI) ────────────────────────────────────────────────────

export function defaultMethod(kind: RcMethodKind, input: string): RcMethod {
  switch (kind) {
    case 'passthrough':
      return { kind, input, invert: false, deadband: 0 };
    case 'analogAdjust':
      return { kind, input, invert: false, rate: 1, deadband: 0.05 };
    case 'dualAxis':
      return { kind, inputAdd: input, inputSub: '', mode: 'absolute', rate: 1 };
    case 'hold':
      return { kind, input, low: -1, high: 1 };
    case 'toggle':
      return { kind, input, positions: evenPositions(3) };
    case 'buttonStep':
      return { kind, inputUp: input, inputDown: '', steps: 5 };
    case 'buttonAdjust':
      return { kind, inputUp: input, inputDown: '', rate: 1 };
  }
}
