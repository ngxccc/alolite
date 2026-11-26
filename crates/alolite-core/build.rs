//! Build script for `alolite-core`.
//!
//! When the `call` Cargo feature is enabled, links system `opus` and (on Linux)
//! `libpulse-simple` via `pkg-config`.

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_CALL");

    if std::env::var_os("CARGO_FEATURE_CALL").is_some() {
        #[cfg(feature = "call")]
        link_call_media_libs();
    }
}

#[cfg(feature = "call")]
fn link_call_media_libs() {
    pkg_config::Config::new()
        .probe("opus")
        .expect("system library `opus` (libopus-dev) is required when feature `call` is enabled");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        pkg_config::Config::new()
            .probe("libpulse-simple")
            .expect(
                "system library `libpulse-simple` (libpulse-dev) is required on Linux when feature `call` is enabled",
            );
    }
}
