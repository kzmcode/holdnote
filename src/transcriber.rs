//! whisper.cpp (`whisper-cli`) discovery, model management and transcription.
//! OWNER: transcriber/markdown agent. The model is only loaded inside the child process,
//! which exits after each job (nothing resident while idle).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// "base" | "small"
    pub name: &'static str,
    /// e.g. "ggml-base.bin"
    pub file_name: &'static str,
    pub url: &'static str,
    pub size_bytes: u64,
    /// Lower-case hex SHA-256 of the file.
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStatus {
    Missing,
    Present,
    /// Exists but size/hash mismatch.
    Corrupt,
}

#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    #[error("whisper-cli が見つかりません")]
    CliNotFound,
    #[error("音声認識モデルがありません")]
    ModelMissing,
    #[error("文字起こしに失敗しました: {0}")]
    Failed(String),
    #[error("音声が認識できませんでした（無音）")]
    Empty,
}

pub fn supported_models() -> &'static [ModelInfo] {
    todo!()
}
pub fn model_info(name: &str) -> Option<&'static ModelInfo> {
    let _ = name;
    todo!()
}
pub fn model_path(name: &str) -> PathBuf {
    let _ = name;
    todo!()
}
/// Cheap check (size only) unless `verify_hash`.
pub fn model_status(name: &str, verify_hash: bool) -> ModelStatus {
    let _ = (name, verify_hash);
    todo!()
}

/// Download to a temp file, verify SHA-256, then atomically rename into `paths::models_dir()`.
/// The previous model file is untouched on failure. Blocking; call from a background thread.
pub fn download_model(name: &str, progress: &dyn Fn(u64, u64)) -> anyhow::Result<PathBuf> {
    let _ = (name, progress);
    todo!()
}

/// Search order: explicit setting → `<App>.app/Contents/Resources/bin/whisper-cli` →
/// /opt/homebrew/bin/whisper-cli → /usr/local/bin/whisper-cli → $PATH.
pub fn find_whisper_cli(explicit: Option<&str>) -> Option<PathBuf> {
    let _ = explicit;
    todo!()
}

/// Run whisper-cli on a 16 kHz mono WAV and return the verbatim transcript (trimmed, no timestamps).
/// Blocking; the child process is killed if it runs longer than `timeout`.
pub fn transcribe(
    cli: &Path,
    model: &Path,
    wav: &Path,
    language: &str,
    timeout: std::time::Duration,
) -> Result<String, TranscribeError> {
    let _ = (cli, model, wav, language, timeout);
    todo!()
}
