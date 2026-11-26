//! Asynchronous 3-channel WAV debug dumper for tuning call AEC/NS/AGC.
//!
//! Activated only when `ALOLITE_CALL_DUMP_WAV` is set (`1`/`true`/`yes` or a directory path).
//! Captures 16 kHz 16-bit 3-channel PCM (`ch0 = mic_raw`, `ch1 = mic_proc`, `ch2 = speaker`)
//! via a bounded `try_send` channel so audio capture/playout threads never block on disk I/O.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::thread::JoinHandle;

use super::{FRAME_SAMPLES, SAMPLE_RATE_HZ};

const WAV_CHANNELS: u16 = 3;
const WAV_HEADER_LEN: u64 = 44;
const CHANNEL_CAPACITY: usize = 256;
const MAX_SKEW_FRAMES: usize = 4;

/// PCM frame sent from capture/playout threads to the background WAV writer.
// Inline fixed arrays avoid heap allocation on the 20 ms real-time audio threads.
#[allow(clippy::large_enum_variant)]
pub enum WavDumpMsg {
    /// One 20 ms microphone frame before (`raw`) and after (`proc`) APM + AGC.
    Capture {
        raw: [i16; FRAME_SAMPLES],
        proc: [i16; FRAME_SAMPLES],
    },
    /// One 20 ms speaker render frame fed to PulsePlayback and AEC reverse stream.
    Render([i16; FRAME_SAMPLES]),
}

/// Resolve the output `.wav` path from `ALOLITE_CALL_DUMP_WAV`, if enabled.
#[must_use]
pub fn resolve_dump_path_from_value(
    val: Option<&str>,
    call_id: u32,
    default_dir: &Path,
) -> Option<PathBuf> {
    let raw = val?.trim();
    if raw.is_empty()
        || raw.eq_ignore_ascii_case("0")
        || raw.eq_ignore_ascii_case("false")
        || raw.eq_ignore_ascii_case("no")
    {
        return None;
    }
    let dir = if raw == "1" || raw.eq_ignore_ascii_case("true") || raw.eq_ignore_ascii_case("yes") {
        default_dir.to_path_buf()
    } else {
        PathBuf::from(raw)
    };
    Some(dir.join(format!("call-dump-{call_id}.wav")))
}

/// Start the async WAV dumper if `ALOLITE_CALL_DUMP_WAV` is enabled.
///
/// Returns `(capture_tx, render_tx, writer_join_handle)`. Dropping both senders flushes
/// and finalizes the WAV RIFF header cleanly.
#[must_use]
pub fn start_from_env(
    call_id: u32,
    default_dir: &Path,
) -> Option<(
    SyncSender<WavDumpMsg>,
    SyncSender<WavDumpMsg>,
    JoinHandle<()>,
)> {
    let env_val = std::env::var("ALOLITE_CALL_DUMP_WAV").ok();
    let path = resolve_dump_path_from_value(env_val.as_deref(), call_id, default_dir)?;
    start_at_path(path)
}

/// Start the async WAV dumper writing directly to `path`.
#[must_use]
pub fn start_at_path(
    path: PathBuf,
) -> Option<(
    SyncSender<WavDumpMsg>,
    SyncSender<WavDumpMsg>,
    JoinHandle<()>,
)> {
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            crate::logger::call_debug(&format!(
                "[DEBUG-call] wav_dump_mkdir_failed path={} err={e}",
                path.display()
            ));
            return None;
        }
    }
    let mut file = match File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            crate::logger::call_debug(&format!(
                "[DEBUG-call] wav_dump_open_failed path={} err={e}",
                path.display()
            ));
            return None;
        }
    };
    if let Err(e) = write_wav_header(&mut file, 0) {
        crate::logger::call_debug(&format!(
            "[DEBUG-call] wav_dump_header_failed path={} err={e}",
            path.display()
        ));
        return None;
    }

    crate::logger::call_debug(&format!(
        "[DEBUG-call] wav_dump_enabled path={}",
        path.display()
    ));

    let (tx, rx) = mpsc::sync_channel::<WavDumpMsg>(CHANNEL_CAPACITY);
    let tx_render = tx.clone();

    let handle = std::thread::Builder::new()
        .name("alolite-wav-dump".to_string())
        .spawn(move || {
            let mut writer = BufWriter::new(file);
            let mut cap_q: VecDeque<([i16; FRAME_SAMPLES], [i16; FRAME_SAMPLES])> = VecDeque::new();
            let mut ren_q: VecDeque<[i16; FRAME_SAMPLES]> = VecDeque::new();
            let mut data_bytes: u32 = 0;
            let silence = [0i16; FRAME_SAMPLES];

            while let Ok(msg) = rx.recv() {
                match msg {
                    WavDumpMsg::Capture { raw, proc } => cap_q.push_back((raw, proc)),
                    WavDumpMsg::Render(ren) => ren_q.push_back(ren),
                }

                while !cap_q.is_empty() && !ren_q.is_empty() {
                    if let (Some((raw, proc)), Some(ren)) = (cap_q.pop_front(), ren_q.pop_front()) {
                        data_bytes = data_bytes.saturating_add(write_interleaved_frame(
                            &mut writer,
                            &raw,
                            &proc,
                            &ren,
                        ));
                    }
                }
                while cap_q.len() > MAX_SKEW_FRAMES {
                    if let Some((raw, proc)) = cap_q.pop_front() {
                        data_bytes = data_bytes.saturating_add(write_interleaved_frame(
                            &mut writer,
                            &raw,
                            &proc,
                            &silence,
                        ));
                    }
                }
                while ren_q.len() > MAX_SKEW_FRAMES {
                    if let Some(ren) = ren_q.pop_front() {
                        data_bytes = data_bytes.saturating_add(write_interleaved_frame(
                            &mut writer,
                            &silence,
                            &silence,
                            &ren,
                        ));
                    }
                }
            }

            // Drain any remaining tail frames on teardown.
            while let Some((raw, proc)) = cap_q.pop_front() {
                let ren = ren_q.pop_front().unwrap_or(silence);
                data_bytes = data_bytes.saturating_add(write_interleaved_frame(
                    &mut writer,
                    &raw,
                    &proc,
                    &ren,
                ));
            }
            while let Some(ren) = ren_q.pop_front() {
                data_bytes = data_bytes.saturating_add(write_interleaved_frame(
                    &mut writer,
                    &silence,
                    &silence,
                    &ren,
                ));
            }

            if let Ok(mut inner) = writer.into_inner() {
                let _ = inner.seek(SeekFrom::Start(0));
                let _ = write_wav_header(&mut inner, data_bytes);
                let _ = inner.flush();
            }
        })
        .ok()?;

    Some((tx, tx_render, handle))
}

fn write_interleaved_frame<W: Write>(
    writer: &mut W,
    raw: &[i16; FRAME_SAMPLES],
    proc: &[i16; FRAME_SAMPLES],
    ren: &[i16; FRAME_SAMPLES],
) -> u32 {
    let mut buf = [0u8; FRAME_SAMPLES * WAV_CHANNELS as usize * 2];
    for i in 0..FRAME_SAMPLES {
        let off = i * 6;
        buf[off..off + 2].copy_from_slice(&raw[i].to_le_bytes());
        buf[off + 2..off + 4].copy_from_slice(&proc[i].to_le_bytes());
        buf[off + 4..off + 6].copy_from_slice(&ren[i].to_le_bytes());
    }
    if writer.write_all(&buf).is_ok() {
        u32::try_from(buf.len()).unwrap_or(0)
    } else {
        0
    }
}

fn write_wav_header<W: Write>(writer: &mut W, data_bytes: u32) -> std::io::Result<()> {
    let byte_rate = SAMPLE_RATE_HZ * u32::from(WAV_CHANNELS) * 2;
    let block_align = WAV_CHANNELS * 2;
    let riff_size = (WAV_HEADER_LEN as u32 - 8).saturating_add(data_bytes);

    let mut hdr = [0u8; 44];
    hdr[0..4].copy_from_slice(b"RIFF");
    hdr[4..8].copy_from_slice(&riff_size.to_le_bytes());
    hdr[8..12].copy_from_slice(b"WAVE");
    hdr[12..16].copy_from_slice(b"fmt ");
    hdr[16..20].copy_from_slice(&16u32.to_le_bytes());
    hdr[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    hdr[22..24].copy_from_slice(&WAV_CHANNELS.to_le_bytes());
    hdr[24..28].copy_from_slice(&SAMPLE_RATE_HZ.to_le_bytes());
    hdr[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    hdr[32..34].copy_from_slice(&block_align.to_le_bytes());
    hdr[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    hdr[36..40].copy_from_slice(b"data");
    hdr[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    writer.write_all(&hdr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_dump_path_respects_disabled_and_custom_dirs() {
        let base = Path::new("/tmp/alolite-test-dir");
        assert_eq!(resolve_dump_path_from_value(None, 42, base), None);
        assert_eq!(resolve_dump_path_from_value(Some("0"), 42, base), None);
        assert_eq!(resolve_dump_path_from_value(Some("false"), 42, base), None);
        assert_eq!(
            resolve_dump_path_from_value(Some("1"), 42, base),
            Some(base.join("call-dump-42.wav"))
        );
        assert_eq!(
            resolve_dump_path_from_value(Some("/var/tmp/dumps"), 99, base),
            Some(PathBuf::from("/var/tmp/dumps/call-dump-99.wav"))
        );
    }

    #[test]
    fn wav_dump_writes_valid_3ch_header_and_interleaved_samples() {
        let temp_dir =
            std::env::temp_dir().join(format!("alolite_wav_test_{}", std::process::id()));
        let wav_path = temp_dir.join("test-3ch.wav");

        let (cap_tx, ren_tx, handle) = start_at_path(wav_path.clone()).expect("start wav dumper");
        let mut raw = [0i16; FRAME_SAMPLES];
        let mut proc = [0i16; FRAME_SAMPLES];
        let mut ren = [0i16; FRAME_SAMPLES];
        raw[0] = 111;
        proc[0] = 222;
        ren[0] = -333;

        cap_tx
            .try_send(WavDumpMsg::Capture { raw, proc })
            .expect("send cap");
        ren_tx.try_send(WavDumpMsg::Render(ren)).expect("send ren");
        drop(cap_tx);
        drop(ren_tx);
        handle.join().expect("join wav dumper");

        let bytes = fs::read(&wav_path).expect("read wav");
        let _ = fs::remove_dir_all(&temp_dir);

        let expected_data = (FRAME_SAMPLES * 3 * 2) as u32;
        assert_eq!(bytes.len(), 44 + expected_data as usize);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 3);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            SAMPLE_RATE_HZ
        );
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            expected_data
        );
        assert_eq!(i16::from_le_bytes([bytes[44], bytes[45]]), 111);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 222);
        assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), -333);
    }
}
