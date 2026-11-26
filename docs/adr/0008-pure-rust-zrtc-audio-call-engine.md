---
status: Accepted
date: 2026-11-26
deciders: ngxccc
---

# 8. Pure-Rust ZRTC Audio Call Engine in `alolite-core`

## Context

AloLite is a Tauri v2 + React desktop client backed by `alolite-core` (Rust). Implementing 1-1 voice calls requires interoperating with Zalo's custom **ZRTC** UDP transport (`0x01`/`0x02` ZRTP control frames, `0x03..0x0f` relay media, `0x7f` direct P2P extension, SRTP `AES-128-CM + HMAC-SHA1-80` with `RocTracker` fallback, and 20/40/60 ms Opus frames).

Two alternative designs were evaluated:

1. **WebView WebRTC Bridge**: Forward signaling/PCM across Tauri IPC to WebKitGTK `RTCPeerConnection` / WebAudio. Rejected because WebRTC cannot speak Zalo's custom UDP framing (`0x01`/`0x7f`), pushing 20 ms PCM frames over IPC introduces jitter and violates `INV-3`, and headless `--api-only` / `--mcp` hosts have no WebView.
2. **Cross-Platform `cpal` Callback Timer vs `libpulse-simple` Hardware Pacing**: Empirical Linux testing (`re-notes/16`) showed that mixing `cpal` / timer-based 20 ms sleeps with PipeWire/Bluetooth quantum wakeups (15–19 ms) causes ~11% over-consumption and starves the jitter buffer into Opus PLC loops.

- **Target Module**: `crates/alolite-core/src/protocol/call/`, `crates/alolite-core/src/services/call_media/`

## Decision

Implement the 1-1 audio call engine entirely in Rust inside `alolite-core`, split across the existing `ADR-0002` boundary:

1. **Pure Protocol Layer (`protocol/call/`)**: Owns HTTP signaling (`11300..11311`, `12006`), ZRTP packet framing, SRTP (`srtpMode: 1`) + `RocTracker` fallback, and the single-call state machine (`CallSession`), with zero audio device or SQLite imports (`INV-5`, `INV-8`).
2. **Media Service Layer (`services/call_media/`)**: Owns the UDP socket loop, `NetEq` jitter buffer (`1920`-sample decode scratch + `carry_pcm` FIFO + in-band FEC), system `libopus` (`pkg-config`, gated behind a Cargo feature), and an `AudioBackend` trait whose primary Linux implementation uses `libpulse-simple` (`tlength = 1920`, blocking hardware sample clock).
3. **Engine Isolation (`INV-7`, `INV-9`)**: `CallManager` lives outside the `CoreEngine::session` mutex and emits `CoreEvent` snapshots to Tauri UI; headless mode never auto-answers or opens the microphone.

## Positive Consequences

- Preserves `ADR-0002` pure-protocol boundaries and `INV-3` non-blocking UI guarantees.
- Eliminates PipeWire/Bluetooth 11% audio drift via hardware sample-clock blocking (`pa_simple_write`) and handles 40/60 ms Opus packets cleanly via `carry_pcm`.
- Avoids CMake source builds for `libopus` by linking the system library via `pkg-config`.

## Negative Consequences / Risks

- Requires `libopus-dev` and `libpulse-dev` on Linux build hosts when the call feature is enabled.
- Supporting Windows (WASAPI) or macOS (CoreAudio) later requires adding platform implementations behind the `AudioBackend` trait.

## Explicit Tradeoffs

- **Pure Rust ZRTC vs WebView WebRTC**: Chose Pure Rust in `alolite-core` to support Zalo's custom UDP `0x01`/`0x7f` framing and headless mode (`--api-only`), accepting system `libopus` FFI binding complexity.
- **`libpulse-simple` vs `cpal` callback**: Chose `libpulse-simple` (`tlength = 1920`) on Linux for hardware sample-clock blocking, trading cross-platform uniformity for zero PipeWire/Bluetooth timing drift.
