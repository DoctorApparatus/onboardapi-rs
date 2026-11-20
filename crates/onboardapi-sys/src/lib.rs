//! Raw FFI to the onboardapi C++ SDK.
//!
//! The C++ API (virtual callback interfaces, `std::string`/`std::vector` structs,
//! templated factories) is not callable from Rust directly. A generated C++ shim
//! (`shim/`) exposes every interface module as an [`obapi_vtable`] and moves all
//! data across the boundary in one small binary format (see `shim/obapi_wire.hpp`).
//!
//! Use the safe `onboardapi` crate instead of this one.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void};

#[rustfmt::skip]
mod generated;
pub use generated::*;

pub const OBAPI_OK: i32 = 0;
pub const OBAPI_ERR_EXCEPTION: i32 = -1;
pub const OBAPI_ERR_UNKNOWN: i32 = -2;
pub const OBAPI_ERR_WIRE: i32 = -3;
pub const OBAPI_ERR_INVALID: i32 = -4;

/// Called from onboardapi threads for every incoming operation.
pub type obapi_dispatch_fn =
    unsafe extern "C" fn(ctx: *mut c_void, op: u32, data: *const u8, len: usize);

/// Mirror of `obapi_vtable` in `shim/obapi.h`.
#[repr(C)]
#[derive(Debug)]
pub struct obapi_vtable {
    pub module: *const c_char,
    pub role: *const c_char,
    pub create: unsafe extern "C" fn(
        domain: i32,
        name: *const c_char,
        name_len: usize,
        ctx: *mut c_void,
        dispatch: Option<obapi_dispatch_fn>,
        out: *mut *mut c_void,
    ) -> i32,
    pub send:
        unsafe extern "C" fn(handle: *mut c_void, op: u32, data: *const u8, len: usize) -> i32,
    pub destroy: unsafe extern "C" fn(handle: *mut c_void),
    pub is_valid: unsafe extern "C" fn(handle: *mut c_void) -> bool,
    pub wait_until_connected: Option<unsafe extern "C" fn(handle: *mut c_void) -> i32>,
    pub set_name: Option<
        unsafe extern "C" fn(handle: *mut c_void, name: *const c_char, name_len: usize) -> i32,
    >,
}

// SAFETY: the vtables are immutable statics with 'static string pointers.
unsafe impl Sync for obapi_vtable {}

unsafe extern "C" {
    /// Copies the calling thread's last error into `buf` (truncated) and returns its full length.
    pub fn obapi_last_error(buf: *mut c_char, cap: usize) -> usize;
}

/// The last error message of the calling thread.
pub fn last_error() -> String {
    let mut buf = vec![0u8; 512];
    // SAFETY: buf is valid for buf.len() bytes.
    let n = unsafe { obapi_last_error(buf.as_mut_ptr().cast(), buf.len()) };
    if n > buf.len() {
        buf.resize(n, 0);
        // SAFETY: as above, with the larger buffer.
        unsafe { obapi_last_error(buf.as_mut_ptr().cast(), buf.len()) };
    }
    buf.truncate(n);
    String::from_utf8_lossy(&buf).into_owned()
}
