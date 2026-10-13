//! Microphone capture via cpal → mono 16 kHz 16-bit PCM WAV.
//! OWNER: recorder/hotkey agent. `Recorder` lives on the main thread (cpal::Stream is !Send).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::events::EventSink;

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

pub struct Recorder {
    // implementation detail
}

impl Recorder {
    /// `sink` receives `AppEvent::RecorderFailed` if the stream errors (device unplugged...).
    pub fn new(sink: EventSink) -> Self {
        let _ = sink;
        todo!()
    }
    /// Open the default input device and start writing a WAV to `out_path`.
    /// The input stream exists ONLY while recording (REC-07). Must return quickly (<100ms typical).
    pub fn start(&mut self, out_path: &Path) -> Result<(), RecorderError> {
        let _ = out_path;
        todo!()
    }
    /// Stop, flush and finalize the WAV.
    pub fn stop(&mut self) -> Result<RecordingResult, RecorderError> {
        todo!()
    }
    /// Stop and delete the partial file.
    pub fn cancel(&mut self) {
        todo!()
    }
    pub fn is_recording(&self) -> bool {
        todo!()
    }
    pub fn elapsed(&self) -> Duration {
        todo!()
    }
    /// Smoothed input level 0.0..=1.0 for the overlay meter (cheap; called ≤10 fps).
    pub fn level(&self) -> f32 {
        todo!()
    }
}

/// Microphone TCC authorization (AVCaptureDevice authorizationStatusForMediaType:audio).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicPermission {
    Granted,
    Denied,
    NotDetermined,
}

pub fn mic_permission() -> MicPermission {
    todo!()
}

/// Fix the header of a WAV left behind by a crash; returns its duration. Err if unrecoverable.
pub fn repair_wav(path: &Path) -> anyhow::Result<Duration> {
    let _ = path;
    todo!()
}
