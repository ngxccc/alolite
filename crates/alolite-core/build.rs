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

    compile_webrtc_apm();
}

#[cfg(feature = "call")]
fn compile_webrtc_apm() {
    println!("cargo:rerun-if-changed=src/services/call_media/c_apm.cpp");

    let has_webrtc = std::process::Command::new("pkg-config")
        .args(["--exists", "webrtc-audio-processing-1"])
        .status()
        .is_ok_and(|s| s.success());

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .warnings(false)
        .file("src/services/call_media/c_apm.cpp");

    if has_webrtc {
        let cflags_out = std::process::Command::new("pkg-config")
            .args(["--cflags", "webrtc-audio-processing-1"])
            .output()
            .ok();
        let mut added_include = false;
        if let Some(out) = cflags_out.filter(|o| o.status.success()) {
            for flag in String::from_utf8_lossy(&out.stdout).split_whitespace() {
                if let Some(inc) = flag.strip_prefix("-I") {
                    build.include(inc);
                    if let Some(parent) = std::path::Path::new(inc).parent() {
                        build.include(parent);
                    }
                    added_include = true;
                }
            }
        }
        if !added_include {
            build.include("/usr/include/webrtc-audio-processing-1");
        }

        build
            .define("HAS_WEBRTC_APM", None)
            .define("WEBRTC_POSIX", None)
            .define("WEBRTC_LIBRARY_IMPL", None)
            .define("NOMINMAX", None);
        build.compile("alolite_apm");

        let libdir = std::process::Command::new("pkg-config")
            .args(["--variable=libdir", "webrtc-audio-processing-1"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map_or_else(String::new, |o| {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            });
        let is_static = !libdir.is_empty()
            && std::path::Path::new(&libdir)
                .join("libwebrtc-audio-processing-1.a")
                .exists()
            && !std::path::Path::new(&libdir)
                .join("libwebrtc-audio-processing-1.so")
                .exists();

        let libs_args = if is_static {
            vec!["--libs", "--static", "webrtc-audio-processing-1"]
        } else {
            vec!["--libs", "webrtc-audio-processing-1"]
        };
        let libs_out = std::process::Command::new("pkg-config")
            .args(&libs_args)
            .output()
            .ok();
        let mut added_lib = false;
        if let Some(out) = libs_out.filter(|o| o.status.success()) {
            for flag in String::from_utf8_lossy(&out.stdout).split_whitespace() {
                if let Some(dir) = flag.strip_prefix("-L") {
                    println!("cargo:rustc-link-search=native={dir}");
                } else if let Some(lib) = flag.strip_prefix("-l") {
                    if is_static && lib == "webrtc-audio-processing-1" {
                        println!("cargo:rustc-link-lib=static={lib}");
                    } else {
                        println!("cargo:rustc-link-lib={lib}");
                    }
                    added_lib = true;
                }
            }
        }
        if !added_lib {
            println!("cargo:rustc-link-lib=webrtc-audio-processing-1");
        }
    } else {
        println!(
            "cargo:warning=webrtc-audio-processing-1 not found via pkg-config; compiling APM stubs"
        );
        build.compile("alolite_apm");
    }
}
