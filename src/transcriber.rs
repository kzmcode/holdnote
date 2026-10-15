//! whisper.cpp (`whisper-cli`) discovery, model management and transcription.
//! OWNER: transcriber/markdown agent. The model is only loaded inside the child process,
//! which exits after each job (nothing resident while idle).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};
use sha2::{Digest, Sha256};

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

// Size / SHA-256 (LFS oid) from https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/main
static MODELS: [ModelInfo; 2] = [
    ModelInfo {
        name: "base",
        file_name: "ggml-base.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        size_bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    ModelInfo {
        name: "small",
        file_name: "ggml-small.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        size_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
];

pub fn supported_models() -> &'static [ModelInfo] {
    &MODELS
}

pub fn model_info(name: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.name == name)
}

/// Destination path of a model inside `paths::models_dir()` (unknown names fall back to the "base" file name).
pub fn model_path(name: &str) -> PathBuf {
    let file = model_info(name).map(|m| m.file_name).unwrap_or("ggml-base.bin");
    crate::paths::models_dir().join(file)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Cheap check (size only) unless `verify_hash`.
pub fn model_status(name: &str, verify_hash: bool) -> ModelStatus {
    let Some(info) = model_info(name) else {
        return ModelStatus::Missing;
    };
    let path = model_path(name);
    let Ok(meta) = std::fs::metadata(&path) else {
        return ModelStatus::Missing;
    };
    if !meta.is_file() {
        return ModelStatus::Missing;
    }
    if meta.len() != info.size_bytes {
        return ModelStatus::Corrupt;
    }
    if verify_hash {
        match hash_file(&path) {
            Ok(h) if h == info.sha256 => {}
            _ => return ModelStatus::Corrupt,
        }
    }
    ModelStatus::Present
}

/// Download to a temp file, verify SHA-256, then atomically rename into `paths::models_dir()`.
/// The previous model file is untouched on failure. Blocking; call from a background thread.
pub fn download_model(name: &str, progress: &dyn Fn(u64, u64)) -> anyhow::Result<PathBuf> {
    let info = model_info(name).ok_or_else(|| anyhow!("unknown model: {name}"))?;
    let dest = model_path(name);
    let dir = dest.parent().ok_or_else(|| anyhow!("invalid models dir"))?;
    std::fs::create_dir_all(dir).context("create models dir")?;
    let part = dir.join(format!("{}.part", info.file_name));
    match download_to_part(info, &part, progress) {
        Ok(()) => {
            if let Err(e) = std::fs::rename(&part, &dest) {
                let _ = std::fs::remove_file(&part);
                return Err(anyhow!(e).context("rename model into place"));
            }
            if let Ok(d) = std::fs::File::open(dir) {
                let _ = d.sync_all();
            }
            Ok(dest)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn download_to_part(
    info: &ModelInfo,
    part: &Path,
    progress: &dyn Fn(u64, u64),
) -> anyhow::Result<()> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(None::<Duration>)
        .build()
        .context("http client")?;
    let mut resp = client
        .get(info.url)
        .send()
        .context("model download request failed")?;
    if !resp.status().is_success() {
        return Err(anyhow!("model download failed: HTTP {}", resp.status()));
    }
    let total = info.size_bytes;
    let mut file = std::fs::File::create(part).context("create .part file")?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done: u64 = 0;
    let mut last = Instant::now();
    progress(0, total);
    loop {
        let n = resp.read(&mut buf).context("model download interrupted")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).context("write model")?;
        done += n as u64;
        if done > total {
            return Err(anyhow!("model download larger than expected"));
        }
        if last.elapsed() >= Duration::from_millis(100) {
            progress(done, total);
            last = Instant::now();
        }
    }
    progress(done, total);
    if done != total {
        return Err(anyhow!("model size mismatch: got {done}, expected {total}"));
    }
    if hex(&hasher.finalize()) != info.sha256 {
        return Err(anyhow!("model SHA-256 mismatch"));
    }
    file.flush()?;
    file.sync_all().context("fsync model")?;
    Ok(())
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn bundled_cli() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos = exe.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    if contents.parent()?.extension()? != "app" {
        return None;
    }
    Some(contents.join("Resources/bin/whisper-cli"))
}

/// Search order: explicit setting → `<App>.app/Contents/Resources/bin/whisper-cli` →
/// /opt/homebrew/bin/whisper-cli → /usr/local/bin/whisper-cli → $PATH.
pub fn find_whisper_cli(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(e) = explicit.map(str::trim).filter(|e| !e.is_empty()) {
        let p = PathBuf::from(e);
        if is_executable(&p) {
            return Some(p);
        }
    }
    let mut cands: Vec<PathBuf> = Vec::new();
    cands.extend(bundled_cli());
    cands.push("/opt/homebrew/bin/whisper-cli".into());
    cands.push("/usr/local/bin/whisper-cli".into());
    if let Some(path) = std::env::var_os("PATH") {
        cands.extend(std::env::split_paths(&path).map(|d| d.join("whisper-cli")));
    }
    cands.into_iter().find(|p| is_executable(p))
}

/// Non-speech artifacts whisper emits; matched case-insensitively.
const ARTIFACTS: &[&str] = &[
    "[blank_audio]", "[ blank_audio ]", "[silence]", "[ silence ]", "[no speech]", "[music]",
    "[ music ]", "(music)", "[音楽]", "(音楽)", "[拍手]", "(拍手)", "[無音]", "(無音)",
    "[沈黙]", "(沈黙)", "[雑音]", "(雑音)", "♪",
];

/// Trim and drop non-speech artifacts when they make up the whole output.
/// Returns `None` if nothing meaningful remains. Other text is kept verbatim.
pub fn clean_transcript(raw: &str) -> Option<String> {
    let mut rest = raw.to_lowercase();
    for a in ARTIFACTS {
        rest = rest.replace(a, "");
    }
    if rest.trim().is_empty() {
        return None;
    }
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !ARTIFACTS.contains(&l.to_lowercase().as_str()))
        .collect();
    Some(lines.join("\n"))
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
    use std::process::{Command, Stdio};
    if !cli.exists() {
        return Err(TranscribeError::CliNotFound);
    }
    if !model.is_file() {
        return Err(TranscribeError::ModelMissing);
    }
    // Write the transcript into the app's private (0700) data dir, not the shared temp dir.
    let prefix = crate::paths::tmp_dir().join(format!("holdnote-{}", uuid::Uuid::new_v4()));
    let txt = PathBuf::from(format!("{}.txt", prefix.display()));

    let mut child = Command::new(cli)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(wav)
        .arg("-l")
        .arg(language)
        .arg("-nt")
        .arg("-np")
        .arg("-otxt")
        .arg("-of")
        .arg(&prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TranscribeError::CliNotFound
            } else {
                TranscribeError::Failed(format!("起動できません: {e}"))
            }
        })?;

    // Drain stderr on a thread (keeping only the tail) so the child never blocks on a full pipe.
    let stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut tail: Vec<u8> = Vec::new();
        if let Some(mut s) = stderr {
            let mut buf = [0u8; 8192];
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > 4096 {
                    let cut = tail.len() - 2048;
                    tail.drain(..cut);
                }
            }
        }
        tail
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    let _ = std::fs::remove_file(&txt);
                    return Err(TranscribeError::Failed(format!(
                        "タイムアウト（{}秒）",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = std::fs::remove_file(&txt);
                return Err(TranscribeError::Failed(format!("待機に失敗: {e}")));
            }
        }
    };
    let tail = reader.join().unwrap_or_default();

    if !status.success() {
        let _ = std::fs::remove_file(&txt);
        let t = String::from_utf8_lossy(&tail);
        let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
        let last = lines[lines.len().saturating_sub(5)..].join(" | ");
        return Err(TranscribeError::Failed(format!(
            "終了コード {}: {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into()),
            last
        )));
    }

    let raw = std::fs::read(&txt);
    let _ = std::fs::remove_file(&txt);
    let raw = raw.map_err(|e| TranscribeError::Failed(format!("出力を読めません: {e}")))?;
    clean_transcript(&String::from_utf8_lossy(&raw)).ok_or(TranscribeError::Empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_table_sane() {
        for m in supported_models() {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
            assert!(m.size_bytes > 1_000_000);
            assert!(m.url.ends_with(m.file_name));
            assert!(!m.file_name.contains(".en"));
            assert_eq!(model_info(m.name).unwrap().file_name, m.file_name);
        }
        assert!(model_info("base").is_some() && model_info("small").is_some());
        assert!(model_info("nope").is_none());
    }

    #[test]
    fn cleaning() {
        assert_eq!(clean_transcript(" [BLANK_AUDIO]\n"), None);
        assert_eq!(clean_transcript("(音楽)\n[音楽]"), None);
        assert_eq!(clean_transcript("  \n "), None);
        assert_eq!(clean_transcript("[BLANK_AUDIO] [BLANK_AUDIO]"), None);
        assert_eq!(
            clean_transcript("\n こんにちは。 \n(音楽)\n続き\n").as_deref(),
            Some("こんにちは。\n続き")
        );
        assert_eq!(clean_transcript("(笑) ですね").as_deref(), Some("(笑) ですね"));
    }

    #[test]
    fn explicit_cli_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("hn-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("whisper-cli");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_ne!(find_whisper_cli(f.to_str()), Some(f.clone()));
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_whisper_cli(f.to_str()), Some(f.clone()));
        let bogus = "/nonexistent/whisper-cli";
        assert_ne!(find_whisper_cli(Some(bogus)), Some(PathBuf::from(bogus)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
