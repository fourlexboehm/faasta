// In a shared `ffi_types.rs` (or similar) that both crates can agree upon:

use std::os::raw::{c_char, c_uchar};

#[repr(C)]
pub struct KeyValuePair {
    pub key: *const c_char,
    pub value: *const c_char,
}

#[repr(C)]
pub struct RequestInfo {
    pub method: *const c_char,
    pub uri: *const c_char,
    pub path: *const c_char,
    pub query: *const c_char,

    pub headers: *const KeyValuePair,
    pub headers_len: usize,

    // We could store a pointer to the body bytes and its length
    pub body: *const c_uchar,
    pub body_len: usize,
}

#[repr(C)]
pub struct ResponseInfo {
    // Typically, we store numeric status code in a field
    pub status_code: u16,

    // Pointers to allocated strings
    pub body: *const c_char,
    pub body_len: usize,

    // If we want to also return headers, we’d do similarly:
    // pub headers: *const KeyValuePair,
    // pub headers_len: usize,
}