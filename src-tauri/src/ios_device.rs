// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! The hardware model identifier ("iPad13,4") for the startup log; the simulator reports the host
//! architecture instead.

use std::ffi::{c_char, c_int, c_void, CStr};

extern "C" {
    fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
}

pub fn log_model() {
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    // SAFETY: `buf`/`len` describe a writable buffer; `hw.machine` is a NUL-terminated string.
    let rc = unsafe {
        sysctlbyname(
            c"hw.machine".as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    let model = (rc == 0)
        .then(|| CStr::from_bytes_until_nul(&buf).ok())
        .flatten()
        .and_then(|c| c.to_str().ok())
        .unwrap_or("?");
    log::warn!("[ios] device model={model}");
}
