// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! The P-touch Cube over IOBluetooth RFCOMM, via the Swift shim in
//! swift/ptbt.swift (see build.rs). Only usable from the process main
//! thread: openRFCOMMChannelSync fails with kIOReturnError elsewhere,
//! and delegate data callbacks are pumped on the main runloop.

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;

use crate::{Error, Result, Transport};

extern "C" {
    fn ptbt_list(buf: *mut c_char, cap: usize) -> isize;
    fn ptbt_open(name: *const c_char, err: *mut c_char, err_cap: usize) -> *mut c_void;
    fn ptbt_write(
        handle: *mut c_void,
        data: *const u8,
        len: usize,
        err: *mut c_char,
        err_cap: usize,
    ) -> i32;
    fn ptbt_read(handle: *mut c_void, out: *mut u8, cap: usize, timeout_s: f64) -> isize;
    fn ptbt_drain(handle: *mut c_void);
    fn ptbt_close(handle: *mut c_void);
}

fn err_buf_to_string(buf: &[u8]) -> String {
    CStr::from_bytes_until_nul(buf)
        .map(|c| c.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown Bluetooth error".into())
}

/// Names of the paired PT-P300BT devices, sorted.
pub fn paired_cubes() -> Vec<String> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe { ptbt_list(buf.as_mut_ptr() as *mut c_char, buf.len()) };
    if n <= 0 {
        return Vec::new();
    }
    String::from_utf8_lossy(&buf[..n as usize])
        .split('\n')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

pub struct BtTransport {
    handle: *mut c_void,
    pub device_name: String,
}

impl BtTransport {
    /// Dial the paired Cube called `name`, or the first one.
    pub fn open(name: Option<&str>) -> Result<BtTransport> {
        let cname = name.map(|s| CString::new(s).expect("printer name with NUL"));
        let mut err = vec![0u8; 512];
        let handle = unsafe {
            ptbt_open(
                cname.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if handle.is_null() {
            return Err(Error(err_buf_to_string(&err)));
        }
        // The Swift side picks the first paired Cube (sorted by name)
        // when no name is given — mirror that choice for printer_id.
        let device_name = name
            .map(str::to_owned)
            .or_else(|| paired_cubes().into_iter().next())
            .unwrap_or_else(|| "PT-P300BT".into());
        Ok(BtTransport {
            handle,
            device_name,
        })
    }
}

impl Transport for BtTransport {
    fn write(&mut self, data: &[u8]) -> Result<()> {
        let mut err = vec![0u8; 512];
        let ret = unsafe {
            ptbt_write(
                self.handle,
                data.as_ptr(),
                data.len(),
                err.as_mut_ptr() as *mut c_char,
                err.len(),
            )
        };
        if ret != 0 {
            return Err(Error(err_buf_to_string(&err)));
        }
        Ok(())
    }

    fn read(&mut self, max: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; max];
        let n = unsafe { ptbt_read(self.handle, buf.as_mut_ptr(), max, 0.2) };
        if n < 0 {
            return Err(Error("Bluetooth read failed".into()));
        }
        buf.truncate(n as usize);
        Ok(buf)
    }

    fn drain(&mut self) -> Result<()> {
        unsafe { ptbt_drain(self.handle) };
        Ok(())
    }
}

impl Drop for BtTransport {
    fn drop(&mut self) {
        unsafe { ptbt_close(self.handle) };
    }
}
