//! Common utilities: timestamps, storage paths, logging, and terminal input.

pub mod clock;
pub mod data_dir;
pub mod logger;
pub mod ui;

/// Encode a byte slice as lowercase hexadecimal.
pub fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}
