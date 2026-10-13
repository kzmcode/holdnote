//! Filesystem locations under ~/Library/Application Support/HoldNote.

use std::path::PathBuf;

pub const APP_NAME: &str = "HoldNote";
pub const BUNDLE_ID: &str = "io.github.holdnote.HoldNote";

/// `~/Library/Application Support/HoldNote` (created on demand, mode 0700).
pub fn app_support_dir() -> PathBuf {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".")) // ~/Library/Application Support on macOS
        .join(APP_NAME);
    ensure_private_dir(&dir);
    dir
}

fn sub(name: &str) -> PathBuf {
    let dir = app_support_dir().join(name);
    ensure_private_dir(&dir);
    dir
}

/// Recorded WAV files: `<support>/audio/<job_id>.wav`
pub fn audio_dir() -> PathBuf {
    sub("audio")
}
/// Generated markdown files kept locally: `<support>/queue/<file_name>.md`
pub fn markdown_dir() -> PathBuf {
    sub("queue")
}
/// Whisper ggml models: `<support>/models/ggml-<name>.bin`
pub fn models_dir() -> PathBuf {
    sub("models")
}
pub fn logs_dir() -> PathBuf {
    sub("logs")
}
pub fn db_path() -> PathBuf {
    app_support_dir().join("jobs.sqlite3")
}
pub fn settings_path() -> PathBuf {
    app_support_dir().join("settings.toml")
}

fn ensure_private_dir(dir: &std::path::Path) {
    if !dir.exists() {
        let _ = std::fs::create_dir_all(dir);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
    }
}
