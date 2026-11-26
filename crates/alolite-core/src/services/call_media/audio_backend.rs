//! Audio I/O backend abstraction for ZRTC call media.
//!
//! Linux uses `libpulse-simple` with `tlength = 1920` (60 ms) so `pa_simple_write`
//! blocks on the soundcard's hardware sample clock (`re-notes/16` Section 6.4).
//! Headless and default builds use [`NullAudioBackend`].

use anyhow::Result;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::FRAME_SAMPLES;
#[cfg(all(feature = "call", target_os = "linux"))]
use super::{CHANNELS, PLAYBACK_TLENGTH_BYTES, SAMPLE_RATE_HZ};

/// Direction/kind of an audio device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioDeviceKind {
    /// Microphone / audio capture source.
    Input,
    /// Speaker / audio playback sink.
    Output,
}

/// Enumerated audio input (microphone) or output (speaker) device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AudioDeviceInfo {
    /// Backend device identifier (`"default"` for system default).
    pub id: String,
    /// Human-readable device label for UI display.
    pub name: String,
    /// Whether this device is an input (mic) or output (speaker).
    pub kind: AudioDeviceKind,
    /// True if this is the system default device.
    pub is_default: bool,
}

/// Snapshot of available audio input and output devices.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AudioDeviceList {
    /// Available microphone / capture devices.
    pub inputs: Vec<AudioDeviceInfo>,
    /// Available speaker / playback devices.
    pub outputs: Vec<AudioDeviceInfo>,
}

/// Normalize a UI/IPC device identifier for [`PulseAudioBackend::open`].
///
/// Treats `None`, `""`, and `"default"` as `None` (system default device).
pub fn normalize_device_id(id: Option<&str>) -> Option<&str> {
    match id.map(str::trim) {
        None | Some("") | Some("default") => None,
        Some(other) => Some(other),
    }
}

/// Enumerate available audio input and output devices.
///
/// Uses PulseAudio/PipeWire introspection on Linux when the `call` feature is enabled,
/// falling back to [`NullAudioBackend::list_devices`] if the daemon is unreachable or
/// on headless/non-`call` builds.
pub fn list_audio_devices() -> AudioDeviceList {
    #[cfg(all(feature = "call", target_os = "linux"))]
    {
        if let Ok(list) = PulseAudioBackend::list_devices() {
            if !list.inputs.is_empty() || !list.outputs.is_empty() {
                return AudioDeviceList {
                    inputs: if list.inputs.is_empty() {
                        NullAudioBackend::list_devices().inputs
                    } else {
                        list.inputs
                    },
                    outputs: if list.outputs.is_empty() {
                        NullAudioBackend::list_devices().outputs
                    } else {
                        list.outputs
                    },
                };
            }
        }
    }
    NullAudioBackend::list_devices()
}

/// Synchronous 20 ms frame-based audio I/O backend.
///
/// Implementations must run on a dedicated audio thread, never on the UI thread (`INV-3`, `INV-9`).
pub trait AudioBackend: Send {
    /// Human-readable backend identifier (e.g. `"pulse-simple"`, `"null"`).
    fn name(&self) -> &'static str;

    /// Read one 20 ms mono PCM frame (`320` samples at 16 kHz) from the microphone.
    ///
    /// # Errors
    /// Returns an error if `pcm.len() != FRAME_SAMPLES` or the underlying audio stream fails.
    fn read_frame(&mut self, pcm: &mut [i16]) -> Result<()>;

    /// Write one 20 ms mono PCM frame (`320` samples at 16 kHz) to the speaker.
    ///
    /// # Errors
    /// Returns an error if `pcm.len() != FRAME_SAMPLES` or the underlying audio stream fails.
    fn write_frame(&mut self, pcm: &[i16]) -> Result<()>;
}

/// No-op audio backend for headless (`--mcp` / `--api-only`) and default non-`call` builds.
#[derive(Debug, Default)]
pub struct NullAudioBackend {
    frames_written: u64,
    frames_read: u64,
}

impl NullAudioBackend {
    /// Create a new silent/discarding audio backend.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of 20 ms frames written via [`AudioBackend::write_frame`].
    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// Number of 20 ms frames read via [`AudioBackend::read_frame`].
    pub fn frames_read(&self) -> u64 {
        self.frames_read
    }

    /// Return fallback default input/output entries for headless or non-`call` environments.
    pub fn list_devices() -> AudioDeviceList {
        AudioDeviceList {
            inputs: vec![AudioDeviceInfo {
                id: "default".to_string(),
                name: "Micro mặc định hệ thống".to_string(),
                kind: AudioDeviceKind::Input,
                is_default: true,
            }],
            outputs: vec![AudioDeviceInfo {
                id: "default".to_string(),
                name: "Loa mặc định hệ thống".to_string(),
                kind: AudioDeviceKind::Output,
                is_default: true,
            }],
        }
    }
}

impl AudioBackend for NullAudioBackend {
    fn name(&self) -> &'static str {
        "null"
    }

    fn read_frame(&mut self, pcm: &mut [i16]) -> Result<()> {
        if pcm.len() != FRAME_SAMPLES {
            anyhow::bail!(
                "expected {FRAME_SAMPLES} PCM samples for capture, got {}",
                pcm.len()
            );
        }
        pcm.fill(0);
        self.frames_read += 1;
        Ok(())
    }

    fn write_frame(&mut self, pcm: &[i16]) -> Result<()> {
        if pcm.len() != FRAME_SAMPLES {
            anyhow::bail!(
                "expected {FRAME_SAMPLES} PCM samples for playback, got {}",
                pcm.len()
            );
        }
        self.frames_written += 1;
        Ok(())
    }
}

#[cfg(all(feature = "call", target_os = "linux"))]
mod pulse {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_void};

    #[repr(C)]
    struct PaSimple {
        _private: [u8; 0],
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct PaSampleSpec {
        pub format: c_int,
        pub rate: u32,
        pub channels: u8,
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct PaBufferAttr {
        pub maxlength: u32,
        pub tlength: u32,
        pub prebuf: u32,
        pub minreq: u32,
        pub fragsize: u32,
    }

    const PA_SAMPLE_S16LE: c_int = 3;
    const PA_STREAM_PLAYBACK: c_int = 1;
    const PA_STREAM_RECORD: c_int = 2;
    const PA_CONTEXT_READY: c_int = 4;
    const PA_CONTEXT_FAILED: c_int = 5;
    const PA_CONTEXT_TERMINATED: c_int = 6;
    const PA_OPERATION_RUNNING: c_int = 0;

    #[repr(C)]
    struct PaMainloop {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct PaMainloopApi {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct PaContext {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct PaOperation {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct PaDeviceInfoPrefix {
        name: *const c_char,
        _index: u32,
        description: *const c_char,
    }

    #[repr(C)]
    struct PaServerInfoPrefix {
        _user_name: *const c_char,
        _host_name: *const c_char,
        _server_version: *const c_char,
        _server_name: *const c_char,
        _sample_spec: PaSampleSpec,
        default_sink_name: *const c_char,
        default_source_name: *const c_char,
    }

    extern "C" {
        fn pa_mainloop_new() -> *mut PaMainloop;
        fn pa_mainloop_free(m: *mut PaMainloop);
        fn pa_mainloop_get_api(m: *mut PaMainloop) -> *mut PaMainloopApi;
        fn pa_mainloop_iterate(m: *mut PaMainloop, block: c_int, retval: *mut c_int) -> c_int;

        fn pa_context_new(mainloop: *mut PaMainloopApi, name: *const c_char) -> *mut PaContext;
        fn pa_context_unref(c: *mut PaContext);
        fn pa_context_connect(
            c: *mut PaContext,
            server: *const c_char,
            flags: c_int,
            api: *const c_void,
        ) -> c_int;
        fn pa_context_disconnect(c: *mut PaContext);
        fn pa_context_get_state(c: *const PaContext) -> c_int;

        fn pa_context_get_server_info(
            c: *mut PaContext,
            cb: extern "C" fn(*mut PaContext, *const PaServerInfoPrefix, *mut c_void),
            userdata: *mut c_void,
        ) -> *mut PaOperation;
        fn pa_context_get_sink_info_list(
            c: *mut PaContext,
            cb: extern "C" fn(*mut PaContext, *const PaDeviceInfoPrefix, c_int, *mut c_void),
            userdata: *mut c_void,
        ) -> *mut PaOperation;
        fn pa_context_get_source_info_list(
            c: *mut PaContext,
            cb: extern "C" fn(*mut PaContext, *const PaDeviceInfoPrefix, c_int, *mut c_void),
            userdata: *mut c_void,
        ) -> *mut PaOperation;

        fn pa_operation_get_state(o: *const PaOperation) -> c_int;
        fn pa_operation_unref(o: *mut PaOperation);

        fn pa_simple_new(
            server: *const c_char,
            name: *const c_char,
            dir: c_int,
            dev: *const c_char,
            stream_name: *const c_char,
            ss: *const PaSampleSpec,
            map: *const c_void,
            attr: *const PaBufferAttr,
            error: *mut c_int,
        ) -> *mut PaSimple;
        fn pa_simple_free(s: *mut PaSimple);
        fn pa_simple_write(
            s: *mut PaSimple,
            data: *const c_void,
            bytes: usize,
            error: *mut c_int,
        ) -> c_int;
        fn pa_simple_read(
            s: *mut PaSimple,
            data: *mut c_void,
            bytes: usize,
            error: *mut c_int,
        ) -> c_int;
        fn pa_strerror(error: c_int) -> *const c_char;
    }

    fn pa_error_string(err: c_int) -> String {
        // SAFETY: `pa_strerror` returns a static NUL-terminated string for any error code.
        let ptr = unsafe { pa_strerror(err) };
        if ptr.is_null() {
            return format!("PulseAudio error {err}");
        }
        // SAFETY: `ptr` is non-null and NUL-terminated per libpulse contract.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    /// Linux `libpulse-simple` duplex audio backend paced by the hardware sample clock.
    pub struct PulseAudioBackend {
        playback: *mut PaSimple,
        capture: *mut PaSimple,
    }

    // SAFETY: `pa_simple` handles are owned exclusively by `PulseAudioBackend` and
    // moved to the single dedicated audio thread (`INV-9`).
    unsafe impl Send for PulseAudioBackend {}

    impl PulseAudioBackend {
        /// Build the `pa_sample_spec` for 16 kHz mono signed 16-bit LE PCM.
        pub(super) fn sample_spec() -> PaSampleSpec {
            PaSampleSpec {
                format: PA_SAMPLE_S16LE,
                rate: SAMPLE_RATE_HZ,
                channels: CHANNELS as u8,
            }
        }

        /// Build the playback `pa_buffer_attr` with `tlength = 1920` (60 ms) so
        /// `pa_simple_write` blocks on the hardware sample clock.
        pub(super) fn playback_buffer_attr() -> PaBufferAttr {
            PaBufferAttr {
                maxlength: u32::MAX,
                tlength: PLAYBACK_TLENGTH_BYTES,
                prebuf: u32::MAX,
                minreq: u32::MAX,
                fragsize: u32::MAX,
            }
        }

        /// Open duplex PulseAudio/PipeWire playback and capture streams.
        ///
        /// # Errors
        /// Returns an error if the PulseAudio daemon is unreachable or rejects the stream parameters,
        /// or if `dev` contains an interior NUL byte.
        pub fn open(playback_dev: Option<&str>, capture_dev: Option<&str>) -> Result<Self> {
            let app_name = CString::new("AloLite").expect("static string");
            let pb_stream = CString::new("ZRTC Playback").expect("static string");
            let cap_stream = CString::new("ZRTC Capture").expect("static string");
            let pb_dev_c = playback_dev.map(CString::new).transpose()?;
            let cap_dev_c = capture_dev.map(CString::new).transpose()?;

            let ss = Self::sample_spec();
            let pb_attr = Self::playback_buffer_attr();

            let mut err: c_int = 0;
            // SAFETY: All pointers are valid for the duration of `pa_simple_new`, and `err` is initialized.
            let playback = unsafe {
                pa_simple_new(
                    std::ptr::null(),
                    app_name.as_ptr(),
                    PA_STREAM_PLAYBACK,
                    pb_dev_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                    pb_stream.as_ptr(),
                    &ss,
                    std::ptr::null(),
                    &pb_attr,
                    &mut err,
                )
            };
            if playback.is_null() {
                anyhow::bail!(
                    "failed to open PulseAudio playback stream: {}",
                    pa_error_string(err)
                );
            }

            // SAFETY: All pointers are valid for the duration of `pa_simple_new`, and `err` is initialized.
            let capture = unsafe {
                pa_simple_new(
                    std::ptr::null(),
                    app_name.as_ptr(),
                    PA_STREAM_RECORD,
                    cap_dev_c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                    cap_stream.as_ptr(),
                    &ss,
                    std::ptr::null(),
                    std::ptr::null(),
                    &mut err,
                )
            };
            if capture.is_null() {
                // SAFETY: `playback` was successfully created above and must not leak on failure.
                unsafe { pa_simple_free(playback) };
                anyhow::bail!(
                    "failed to open PulseAudio capture stream: {}",
                    pa_error_string(err)
                );
            }

            Ok(Self { playback, capture })
        }

        /// Enumerate PulseAudio/PipeWire sinks (speakers) and non-monitor sources (microphones).
        ///
        /// # Errors
        /// Returns an error if the PulseAudio mainloop/context cannot be initialized or fails to connect.
        pub fn list_devices() -> Result<AudioDeviceList> {
            #[derive(Default)]
            struct CollectState {
                default_sink: Option<String>,
                default_source: Option<String>,
                sinks: Vec<(String, String)>,
                sources: Vec<(String, String)>,
            }

            extern "C" fn on_server_info(
                _ctx: *mut PaContext,
                info: *const PaServerInfoPrefix,
                userdata: *mut c_void,
            ) {
                if info.is_null() || userdata.is_null() {
                    return;
                }
                // SAFETY: `info` and `userdata` are valid for the duration of the callback.
                unsafe {
                    let state = &mut *(userdata.cast::<CollectState>());
                    if !(*info).default_sink_name.is_null() {
                        state.default_sink = Some(
                            CStr::from_ptr((*info).default_sink_name)
                                .to_string_lossy()
                                .into_owned(),
                        );
                    }
                    if !(*info).default_source_name.is_null() {
                        state.default_source = Some(
                            CStr::from_ptr((*info).default_source_name)
                                .to_string_lossy()
                                .into_owned(),
                        );
                    }
                }
            }

            extern "C" fn on_sink_info(
                _ctx: *mut PaContext,
                info: *const PaDeviceInfoPrefix,
                eol: c_int,
                userdata: *mut c_void,
            ) {
                if eol != 0 || info.is_null() || userdata.is_null() {
                    return;
                }
                // SAFETY: `info` and `userdata` are valid for the duration of the callback.
                unsafe {
                    let state = &mut *(userdata.cast::<CollectState>());
                    if (*info).name.is_null() {
                        return;
                    }
                    let id = CStr::from_ptr((*info).name).to_string_lossy().into_owned();
                    let desc = if (*info).description.is_null() {
                        id.clone()
                    } else {
                        CStr::from_ptr((*info).description)
                            .to_string_lossy()
                            .into_owned()
                    };
                    state.sinks.push((id, desc));
                }
            }

            extern "C" fn on_source_info(
                _ctx: *mut PaContext,
                info: *const PaDeviceInfoPrefix,
                eol: c_int,
                userdata: *mut c_void,
            ) {
                if eol != 0 || info.is_null() || userdata.is_null() {
                    return;
                }
                // SAFETY: `info` and `userdata` are valid for the duration of the callback.
                unsafe {
                    let state = &mut *(userdata.cast::<CollectState>());
                    if (*info).name.is_null() {
                        return;
                    }
                    let id = CStr::from_ptr((*info).name).to_string_lossy().into_owned();
                    // Exclude sink monitor virtual sources so only real microphones appear.
                    if id.ends_with(".monitor") {
                        return;
                    }
                    let desc = if (*info).description.is_null() {
                        id.clone()
                    } else {
                        CStr::from_ptr((*info).description)
                            .to_string_lossy()
                            .into_owned()
                    };
                    state.sources.push((id, desc));
                }
            }

            // SAFETY: `pa_mainloop_new` returns an owned mainloop pointer or null.
            let mainloop = unsafe { pa_mainloop_new() };
            if mainloop.is_null() {
                anyhow::bail!("failed to create PulseAudio mainloop");
            }
            let app_name = CString::new("AloLite").expect("static string");
            // SAFETY: `mainloop` is valid and non-null.
            let api = unsafe { pa_mainloop_get_api(mainloop) };
            // SAFETY: `api` and `app_name` are valid.
            let ctx = unsafe { pa_context_new(api, app_name.as_ptr()) };
            if ctx.is_null() {
                // SAFETY: `mainloop` is owned and must be freed before returning.
                unsafe { pa_mainloop_free(mainloop) };
                anyhow::bail!("failed to create PulseAudio context");
            }

            // SAFETY: `ctx` is valid; connecting to default server with no spawn flags.
            let conn_rc = unsafe { pa_context_connect(ctx, std::ptr::null(), 0, std::ptr::null()) };
            if conn_rc < 0 {
                // SAFETY: `ctx` and `mainloop` are valid and owned.
                unsafe {
                    pa_context_unref(ctx);
                    pa_mainloop_free(mainloop);
                }
                anyhow::bail!("failed to connect PulseAudio context");
            }

            let mut ready = false;
            for _ in 0..200 {
                // SAFETY: `mainloop` and `ctx` are valid.
                let st = unsafe {
                    if pa_mainloop_iterate(mainloop, 1, std::ptr::null_mut()) < 0 {
                        PA_CONTEXT_FAILED
                    } else {
                        pa_context_get_state(ctx)
                    }
                };
                if st == PA_CONTEXT_READY {
                    ready = true;
                    break;
                }
                if st == PA_CONTEXT_FAILED || st == PA_CONTEXT_TERMINATED {
                    break;
                }
            }

            if !ready {
                // SAFETY: `ctx` and `mainloop` are valid and owned.
                unsafe {
                    pa_context_disconnect(ctx);
                    pa_context_unref(ctx);
                    pa_mainloop_free(mainloop);
                }
                anyhow::bail!("PulseAudio context did not become ready");
            }

            let mut collect = CollectState::default();
            let userdata = (&mut collect as *mut CollectState).cast::<c_void>();

            // SAFETY: `ctx` is ready and `collect` outlives all operations below.
            let ops = unsafe {
                [
                    pa_context_get_server_info(ctx, on_server_info, userdata),
                    pa_context_get_sink_info_list(ctx, on_sink_info, userdata),
                    pa_context_get_source_info_list(ctx, on_source_info, userdata),
                ]
            };

            for _ in 0..500 {
                let any_running = ops.iter().any(|&op| {
                    // SAFETY: non-null `op` is valid until `pa_operation_unref` below.
                    !op.is_null() && unsafe { pa_operation_get_state(op) } == PA_OPERATION_RUNNING
                });
                if !any_running {
                    break;
                }
                // SAFETY: `mainloop` is valid.
                if unsafe { pa_mainloop_iterate(mainloop, 1, std::ptr::null_mut()) } < 0 {
                    break;
                }
            }

            for op in ops {
                if !op.is_null() {
                    // SAFETY: `op` is owned and non-null.
                    unsafe { pa_operation_unref(op) };
                }
            }
            // SAFETY: `ctx` and `mainloop` are valid and owned.
            unsafe {
                pa_context_disconnect(ctx);
                pa_context_unref(ctx);
                pa_mainloop_free(mainloop);
            }

            let inputs = collect
                .sources
                .into_iter()
                .map(|(id, name)| AudioDeviceInfo {
                    is_default: collect.default_source.as_deref() == Some(id.as_str()),
                    id,
                    name,
                    kind: AudioDeviceKind::Input,
                })
                .collect();
            let outputs = collect
                .sinks
                .into_iter()
                .map(|(id, name)| AudioDeviceInfo {
                    is_default: collect.default_sink.as_deref() == Some(id.as_str()),
                    id,
                    name,
                    kind: AudioDeviceKind::Output,
                })
                .collect();

            Ok(AudioDeviceList { inputs, outputs })
        }
    }

    impl Drop for PulseAudioBackend {
        fn drop(&mut self) {
            if !self.playback.is_null() {
                // SAFETY: `self.playback` is a valid non-null `pa_simple` handle owned by `self`.
                unsafe { pa_simple_free(self.playback) };
            }
            if !self.capture.is_null() {
                // SAFETY: `self.capture` is a valid non-null `pa_simple` handle owned by `self`.
                unsafe { pa_simple_free(self.capture) };
            }
        }
    }

    impl AudioBackend for PulseAudioBackend {
        fn name(&self) -> &'static str {
            "pulse-simple"
        }

        fn read_frame(&mut self, pcm: &mut [i16]) -> Result<()> {
            if pcm.len() != FRAME_SAMPLES {
                anyhow::bail!(
                    "expected {FRAME_SAMPLES} PCM samples for capture, got {}",
                    pcm.len()
                );
            }
            let bytes = std::mem::size_of_val(pcm);
            let mut err: c_int = 0;
            // SAFETY: `self.capture` is valid and `pcm` is writable for `bytes` bytes.
            let rc = unsafe {
                pa_simple_read(
                    self.capture,
                    pcm.as_mut_ptr().cast::<c_void>(),
                    bytes,
                    &mut err,
                )
            };
            if rc < 0 {
                anyhow::bail!("PulseAudio read failed: {}", pa_error_string(err));
            }
            Ok(())
        }

        fn write_frame(&mut self, pcm: &[i16]) -> Result<()> {
            if pcm.len() != FRAME_SAMPLES {
                anyhow::bail!(
                    "expected {FRAME_SAMPLES} PCM samples for playback, got {}",
                    pcm.len()
                );
            }
            let bytes = std::mem::size_of_val(pcm);
            let mut err: c_int = 0;
            // SAFETY: `self.playback` is valid and `pcm` is readable for `bytes` bytes.
            let rc = unsafe {
                pa_simple_write(
                    self.playback,
                    pcm.as_ptr().cast::<c_void>(),
                    bytes,
                    &mut err,
                )
            };
            if rc < 0 {
                anyhow::bail!("PulseAudio write failed: {}", pa_error_string(err));
            }
            Ok(())
        }
    }
}

#[cfg(all(feature = "call", target_os = "linux"))]
pub use pulse::PulseAudioBackend;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_audio_backend_reads_silence_and_tracks_writes() {
        let mut backend = NullAudioBackend::new();
        assert_eq!(backend.name(), "null");

        let mut capture = [42i16; FRAME_SAMPLES];
        backend
            .read_frame(&mut capture)
            .expect("read_frame succeeds");
        assert!(capture.iter().all(|&s| s == 0));
        assert_eq!(backend.frames_read(), 1);

        let playback = [100i16; FRAME_SAMPLES];
        backend
            .write_frame(&playback)
            .expect("write_frame succeeds");
        assert_eq!(backend.frames_written(), 1);
    }

    #[test]
    fn null_audio_backend_rejects_wrong_frame_length() {
        let mut backend = NullAudioBackend::new();
        let mut short = [0i16; 160];
        assert!(backend.read_frame(&mut short).is_err());
        assert!(backend.write_frame(&short).is_err());
    }

    #[cfg(all(feature = "call", target_os = "linux"))]
    #[test]
    fn pulse_playback_attr_uses_hardware_clock_tlength_1920() {
        let attr = PulseAudioBackend::playback_buffer_attr();
        assert_eq!(attr.tlength, 1920);
        assert_eq!(attr.maxlength, u32::MAX);
        assert_eq!(attr.prebuf, u32::MAX);
        assert_eq!(attr.minreq, u32::MAX);
        assert_eq!(attr.fragsize, u32::MAX);

        let ss = PulseAudioBackend::sample_spec();
        assert_eq!(ss.rate, 16_000);
        assert_eq!(ss.channels, 1);
    }

    #[test]
    fn null_audio_backend_lists_default_input_and_output_devices() {
        let list = NullAudioBackend::list_devices();
        assert_eq!(list.inputs.len(), 1);
        assert_eq!(list.outputs.len(), 1);
        assert_eq!(list.inputs[0].id, "default");
        assert_eq!(list.inputs[0].kind, AudioDeviceKind::Input);
        assert!(list.inputs[0].is_default);
        assert_eq!(list.outputs[0].id, "default");
        assert_eq!(list.outputs[0].kind, AudioDeviceKind::Output);
        assert!(list.outputs[0].is_default);
    }

    #[test]
    fn list_audio_devices_returns_at_least_default_fallback() {
        let list = list_audio_devices();
        assert!(!list.inputs.is_empty());
        assert!(!list.outputs.is_empty());
        assert_eq!(normalize_device_id(None), None);
        assert_eq!(normalize_device_id(Some("")), None);
        assert_eq!(normalize_device_id(Some("default")), None);
        assert_eq!(
            normalize_device_id(Some("alsa_input.pci")),
            Some("alsa_input.pci")
        );
    }
}
