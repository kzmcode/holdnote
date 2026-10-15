//! Microphone capture via cpal → mono 16 kHz 16-bit PCM WAV.
//! OWNER: recorder/hotkey agent. `Recorder` lives on the main thread (cpal::Stream is !Send).

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};

use crate::events::{AppEvent, EventSink};

pub const TARGET_RATE: u32 = 16_000;

#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("マイクが見つかりません")]
    NoDevice,
    #[error("マイクへのアクセスが許可されていません")]
    PermissionDenied,
    #[error("すでに録音中です")]
    AlreadyRecording,
    #[error("録音していません")]
    NotRecording,
    #[error("録音エラー: {0}")]
    Other(String),
}

#[derive(Debug, Clone)]
pub struct RecordingResult {
    pub path: PathBuf,
    pub duration: Duration,
    /// Peak absolute amplitude 0.0..=1.0 (used for silence detection).
    pub peak: f32,
    /// Average RMS 0.0..=1.0.
    pub rms: f32,
}

/// Statistics shared between the audio callback (single writer) and the owner.
#[derive(Default)]
struct Stats {
    peak_bits: AtomicU32,
    sumsq_bits: AtomicU64, // f64 bits
    count: AtomicU64,
}

struct Active {
    stream: Stream,
    writer: JoinHandle<Result<u64, String>>,
    stats: Arc<Stats>,
    path: PathBuf,
    started: Instant,
}

pub struct Recorder {
    sink: EventSink,
    level: Arc<AtomicU32>,
    active: Option<Active>,
}

impl Recorder {
    /// `sink` receives `AppEvent::RecorderFailed` if the stream errors (device unplugged...).
    pub fn new(sink: EventSink) -> Self {
        Self { sink, level: Arc::new(AtomicU32::new(0f32.to_bits())), active: None }
    }

    /// Open the default input device and start writing a WAV to `out_path`.
    /// The input stream exists ONLY while recording (REC-07). Must return quickly (<100ms typical).
    pub fn start(&mut self, out_path: &Path) -> Result<(), RecorderError> {
        if self.active.is_some() {
            return Err(RecorderError::AlreadyRecording);
        }
        if mic_permission() == MicPermission::Denied {
            return Err(RecorderError::PermissionDenied);
        }
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(RecorderError::NoDevice)?;
        let supported = device.default_input_config().map_err(|e| RecorderError::Other(e.to_string()))?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.config();
        let in_rate = config.sample_rate;
        let channels = config.channels.max(1) as usize;

        if let Some(dir) = out_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TARGET_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wav = hound::WavWriter::create(out_path, spec).map_err(|e| RecorderError::Other(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<Vec<i16>>();
        let writer = std::thread::Builder::new()
            .name("holdnote-wav-writer".into())
            .spawn(move || writer_loop(wav, rx))
            .map_err(|e| RecorderError::Other(e.to_string()))?;

        let stats = Arc::new(Stats::default());
        self.level.store(0f32.to_bits(), Ordering::Relaxed);
        let proc = Processor::new(in_rate, channels, tx, stats.clone(), self.level.clone());
        let sink = self.sink.clone();
        let err_cb = move |e: cpal::Error| {
            log::error!("audio stream error: {e}");
            sink(AppEvent::RecorderFailed(e.to_string()));
        };

        let built = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, proc, err_cb),
            SampleFormat::I16 => build::<i16>(&device, &config, proc, err_cb),
            SampleFormat::U16 => build::<u16>(&device, &config, proc, err_cb),
            SampleFormat::I8 => build::<i8>(&device, &config, proc, err_cb),
            SampleFormat::U8 => build::<u8>(&device, &config, proc, err_cb),
            SampleFormat::I32 => build::<i32>(&device, &config, proc, err_cb),
            SampleFormat::F64 => build::<f64>(&device, &config, proc, err_cb),
            other => Err(format!("未対応のサンプル形式: {other:?}")),
        };
        let stream = match built {
            Ok(s) => s,
            Err(e) => {
                let _ = writer.join();
                let _ = std::fs::remove_file(out_path);
                return Err(RecorderError::Other(e));
            }
        };
        if let Err(e) = stream.play() {
            drop(stream);
            let _ = writer.join();
            let _ = std::fs::remove_file(out_path);
            return Err(RecorderError::Other(e.to_string()));
        }
        self.active = Some(Active { stream, writer, stats, path: out_path.to_path_buf(), started: Instant::now() });
        Ok(())
    }

    /// Stop, flush and finalize the WAV.
    pub fn stop(&mut self) -> Result<RecordingResult, RecorderError> {
        let a = self.active.take().ok_or(RecorderError::NotRecording)?;
        let Active { stream, writer, stats, path, .. } = a;
        drop(stream); // drops the callback (and its Sender) → writer thread finishes
        let samples = writer
            .join()
            .map_err(|_| RecorderError::Other("writer thread panicked".into()))?
            .map_err(RecorderError::Other)?;
        self.level.store(0f32.to_bits(), Ordering::Relaxed);
        let count = stats.count.load(Ordering::Relaxed);
        let sumsq = f64::from_bits(stats.sumsq_bits.load(Ordering::Relaxed));
        let rms = if count > 0 { (sumsq / count as f64).sqrt() as f32 } else { 0.0 };
        Ok(RecordingResult {
            path,
            duration: Duration::from_secs_f64(samples as f64 / TARGET_RATE as f64),
            peak: f32::from_bits(stats.peak_bits.load(Ordering::Relaxed)).min(1.0),
            rms: rms.min(1.0),
        })
    }

    /// Stop and delete the partial file.
    pub fn cancel(&mut self) {
        if let Some(a) = self.active.take() {
            let Active { stream, writer, path, .. } = a;
            drop(stream);
            let _ = writer.join();
            let _ = std::fs::remove_file(path);
        }
        self.level.store(0f32.to_bits(), Ordering::Relaxed);
    }

    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    pub fn elapsed(&self) -> Duration {
        self.active.as_ref().map(|a| a.started.elapsed()).unwrap_or_default()
    }

    /// Smoothed input level 0.0..=1.0 for the overlay meter (cheap; called ≤10 fps).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut proc: Processor,
    err_cb: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<Stream, String>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config.clone(),
            move |data: &[T], _: &cpal::InputCallbackInfo| proc.process(data.iter().map(|s| f32::from_sample_(*s))),
            err_cb,
            None,
        )
        .map_err(|e| e.to_string())
}

fn writer_loop(
    mut wav: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    rx: Receiver<Vec<i16>>,
) -> Result<u64, String> {
    let mut total: u64 = 0;
    let mut since_flush: u64 = 0;
    let mut err: Option<String> = None;
    while let Ok(chunk) = rx.recv() {
        if err.is_some() {
            continue; // keep draining so the callback never blocks
        }
        for s in &chunk {
            if let Err(e) = wav.write_sample(*s) {
                err = Some(e.to_string());
                break;
            }
        }
        total += chunk.len() as u64;
        since_flush += chunk.len() as u64;
        if since_flush >= TARGET_RATE as u64 {
            since_flush = 0;
            if let Err(e) = wav.flush() {
                err = Some(e.to_string());
            }
        }
    }
    wav.finalize().map_err(|e| e.to_string())?;
    match err {
        Some(e) => Err(e),
        None => Ok(total),
    }
}

/// Runs inside the audio callback: downmix → box-filter resample → level/stats → channel.
struct Processor {
    channels: usize,
    /// Input samples per output sample.
    ratio: f64,
    /// Accumulated weighted sum for the current output sample.
    acc: f64,
    /// Input span (in input samples) accumulated for current output sample.
    filled: f64,
    frame_pos: usize,
    frame_sum: f32,
    tx: Sender<Vec<i16>>,
    stats: Arc<Stats>,
    level: Arc<AtomicU32>,
    smoothed: f32,
}

impl Processor {
    fn new(in_rate: u32, channels: usize, tx: Sender<Vec<i16>>, stats: Arc<Stats>, level: Arc<AtomicU32>) -> Self {
        Self {
            channels,
            ratio: in_rate as f64 / TARGET_RATE as f64,
            acc: 0.0,
            filled: 0.0,
            frame_pos: 0,
            frame_sum: 0.0,
            tx,
            stats,
            level,
            smoothed: 0.0,
        }
    }

    fn process(&mut self, samples: impl Iterator<Item = f32>) {
        let mut out: Vec<i16> = Vec::with_capacity(1024);
        for s in samples {
            self.frame_sum += s;
            self.frame_pos += 1;
            if self.frame_pos < self.channels {
                continue;
            }
            let mono = (self.frame_sum / self.channels as f32) as f64;
            self.frame_pos = 0;
            self.frame_sum = 0.0;
            self.push_mono(mono, &mut out);
        }
        if out.is_empty() {
            return;
        }
        self.update_stats(&out);
        let _ = self.tx.send(out);
    }

    /// Box-filter (area average) resampler; works for any ratio (ratio < 1 → sample-and-hold).
    fn push_mono(&mut self, x: f64, out: &mut Vec<i16>) {
        let mut remaining = 1.0f64; // one input sample of width 1
        while remaining > 1e-12 {
            let need = self.ratio - self.filled;
            if remaining >= need {
                self.acc += x * need;
                let v = (self.acc / self.ratio).clamp(-1.0, 1.0);
                out.push((v * 32767.0).round() as i16);
                remaining -= need;
                self.acc = 0.0;
                self.filled = 0.0;
            } else {
                self.acc += x * remaining;
                self.filled += remaining;
                remaining = 0.0;
            }
        }
    }

    fn update_stats(&mut self, out: &[i16]) {
        let mut peak = 0f32;
        let mut sumsq = 0f64;
        for &s in out {
            let f = s as f32 / 32768.0;
            peak = peak.max(f.abs());
            sumsq += (f as f64) * (f as f64);
        }
        let st = &self.stats;
        if peak > f32::from_bits(st.peak_bits.load(Ordering::Relaxed)) {
            st.peak_bits.store(peak.to_bits(), Ordering::Relaxed);
        }
        let prev = f64::from_bits(st.sumsq_bits.load(Ordering::Relaxed));
        st.sumsq_bits.store((prev + sumsq).to_bits(), Ordering::Relaxed);
        st.count.fetch_add(out.len() as u64, Ordering::Relaxed);

        let rms = (sumsq / out.len() as f64).sqrt() as f32;
        let target = rms_to_level(rms);
        // Fast attack, slow release.
        let coef = if target > self.smoothed { 0.6 } else { 0.15 };
        self.smoothed += (target - self.smoothed) * coef;
        self.level.store(self.smoothed.to_bits(), Ordering::Relaxed);
    }
}

/// Map RMS (linear) to 0..1 using dBFS -60..0.
pub fn rms_to_level(rms: f32) -> f32 {
    if rms <= 1e-6 {
        return 0.0;
    }
    ((20.0 * rms.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Microphone permission (AVFoundation via objc2 dynamic messaging)
// ---------------------------------------------------------------------------

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

/// Name of macOS's current default input device (the one recordings use), e.g. "MacBook Proのマイク".
pub fn default_input_name() -> Option<String> {
    let device = cpal::default_host().default_input_device()?;
    device.description().ok().map(|d| d.name().to_string())
}

/// Microphone TCC authorization (AVCaptureDevice authorizationStatusForMediaType:audio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicPermission {
    Granted,
    Denied,
    NotDetermined,
}

pub fn mic_permission() -> MicPermission {
    use objc2::runtime::AnyClass;
    use objc2_foundation::NSString;
    let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
        return MicPermission::NotDetermined;
    };
    let media = NSString::from_str("soun");
    let status: isize = unsafe { objc2::msg_send![cls, authorizationStatusForMediaType: &*media] };
    match status {
        3 => MicPermission::Granted,
        1 | 2 => MicPermission::Denied,
        _ => MicPermission::NotDetermined,
    }
}

/// Ask macOS to show the microphone permission prompt (calls `cb(false)` if unavailable).
/// `cb` is invoked on an arbitrary thread.
pub fn request_mic_permission(cb: impl FnOnce(bool) + Send + 'static) {
    use block2::RcBlock;
    use objc2::runtime::{AnyClass, Bool};
    use objc2_foundation::NSString;
    let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
        cb(false);
        return;
    };
    let media = NSString::from_str("soun");
    let cb = Mutex::new(Some(cb));
    let block = RcBlock::new(move |granted: Bool| {
        if let Some(f) = cb.lock().ok().and_then(|mut g| g.take()) {
            f(granted.as_bool());
        }
    });
    unsafe {
        let _: () = objc2::msg_send![cls, requestAccessForMediaType: &*media, completionHandler: &*block];
    }
}

// ---------------------------------------------------------------------------
// WAV repair
// ---------------------------------------------------------------------------

/// Fix the header of a WAV left behind by a crash; returns its duration. Err if unrecoverable.
pub fn repair_wav(path: &Path) -> anyhow::Result<Duration> {
    use anyhow::{bail, Context};
    let mut f = OpenOptions::new().read(true).write(true).open(path).context("open wav")?;
    let len = f.metadata()?.len();
    let mut head = vec![0u8; len.min(4096) as usize];
    f.read_exact(&mut head)?;
    if head.len() < 44 || &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        bail!("not a RIFF/WAVE file");
    }
    // Walk chunks to find fmt and data.
    let (mut pos, mut fmt, mut data_off) = (12usize, None, None);
    while pos + 8 <= head.len() {
        let id = &head[pos..pos + 4];
        let size = u32::from_le_bytes(head[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if id == b"fmt " && pos + 8 + 16 <= head.len() {
            let b = &head[pos + 8..];
            let channels = u16::from_le_bytes([b[2], b[3]]) as u64;
            let rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as u64;
            let bits = u16::from_le_bytes([b[14], b[15]]) as u64;
            fmt = Some((channels, rate, bits));
        } else if id == b"data" {
            data_off = Some(pos + 8);
            break;
        }
        pos += 8 + size + (size & 1);
    }
    let (channels, rate, bits) = fmt.context("fmt chunk not found")?;
    let data_off = data_off.context("data chunk not found")? as u64;
    if channels == 0 || rate == 0 || bits == 0 {
        bail!("invalid fmt chunk");
    }
    let frame_bytes = channels * (bits / 8).max(1);
    let mut data_len = len.saturating_sub(data_off);
    data_len -= data_len % frame_bytes;
    if data_len == 0 {
        bail!("no audio data");
    }
    f.set_len(data_off + data_len)?;
    let riff_size = (data_off + data_len - 8) as u32;
    f.seek(SeekFrom::Start(4))?;
    f.write_all(&riff_size.to_le_bytes())?;
    f.seek(SeekFrom::Start(data_off - 4))?;
    f.write_all(&(data_len as u32).to_le_bytes())?;
    f.flush()?;
    Ok(Duration::from_secs_f64((data_len / frame_bytes) as f64 / rate as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("holdnote-test-{}-{name}.wav", std::process::id()))
    }

    #[test]
    fn repair_corrupt_header() {
        let p = tmp("repair");
        let spec = hound::WavSpec { channels: 1, sample_rate: 16000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for i in 0..32000 {
            w.write_sample((i % 100) as i16).unwrap();
        }
        w.finalize().unwrap();
        // corrupt sizes, plus a dangling odd byte
        let mut bytes = std::fs::read(&p).unwrap();
        bytes[4..8].copy_from_slice(&0u32.to_le_bytes());
        bytes[40..44].copy_from_slice(&0u32.to_le_bytes());
        bytes.push(0x7f);
        std::fs::write(&p, &bytes).unwrap();
        let d = repair_wav(&p).unwrap();
        assert_eq!(d, Duration::from_secs(2));
        let r = hound::WavReader::open(&p).unwrap();
        assert_eq!(r.duration(), 32000);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn repair_rejects_garbage() {
        let p = tmp("garbage");
        std::fs::write(&p, b"hello world, not a wav file at all, definitely not.....").unwrap();
        assert!(repair_wav(&p).is_err());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn resample_48k_to_16k_length_and_value() {
        let (tx, rx) = mpsc::channel();
        let mut p = Processor::new(48000, 2, tx, Arc::new(Stats::default()), Arc::new(AtomicU32::new(0)));
        p.process(std::iter::repeat(0.5f32).take(48000 * 2));
        let out = rx.recv().unwrap();
        assert!((out.len() as i64 - 16000).abs() <= 1);
        assert!((out[100] as i32 - 16384).abs() <= 2);
    }

    #[test]
    fn resample_44k1_length() {
        let (tx, rx) = mpsc::channel();
        let mut p = Processor::new(44100, 1, tx, Arc::new(Stats::default()), Arc::new(AtomicU32::new(0)));
        p.process(std::iter::repeat(0.0f32).take(44100));
        assert!((rx.recv().unwrap().len() as i64 - 16000).abs() <= 1);
    }

    #[test]
    fn level_mapping() {
        assert_eq!(rms_to_level(0.0), 0.0);
        assert!((rms_to_level(1.0) - 1.0).abs() < 1e-6);
        assert!(rms_to_level(0.001) < 1e-3);
        assert!(rms_to_level(0.1) > 0.6 && rms_to_level(0.1) < 0.7);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Manual check against the real microphone: `cargo test live_mic -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_mic() {
        let sink: EventSink = std::sync::Arc::new(|e| eprintln!("event: {e:?}"));
        let mut r = Recorder::new(sink);
        eprintln!("permission: {:?}", mic_permission());
        let path = std::env::temp_dir().join("holdnote-live-mic.wav");
        r.start(&path).expect("start");
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(100));
            eprint!("{:.2} ", r.level());
        }
        let res = r.stop().expect("stop");
        eprintln!("\nduration={:?} peak={:.4} rms={:.4} path={}", res.duration, res.peak, res.rms, res.path.display());
    }
}
