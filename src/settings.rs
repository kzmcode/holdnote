//! User settings persisted as TOML at `paths::settings_path()`.
//! Secrets (OAuth tokens) are NOT stored here — see `security`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Hotkey in `global_hotkey::hotkey::HotKey::from_str` syntax, e.g. "control+alt+KeyR".
    pub hotkey: String,
    /// Whisper model name: "base" or "small" (multilingual, never ".en").
    pub model: String,
    /// Transcription language passed to whisper-cli (`-l`).
    pub language: String,
    /// Explicit whisper-cli path; None = auto-detect (bundle → Homebrew).
    pub whisper_cli_path: Option<String>,
    /// Google OAuth "Desktop app" client ID / secret (secret is not treated as confidential).
    pub google_client_id: String,
    pub google_client_secret: String,
    /// Google Cloud project number (the "app id" used by the desktop Picker). Optional.
    pub google_project_number: Option<String>,
    /// Destination Drive folder chosen via the Picker.
    pub drive_folder_id: Option<String>,
    pub drive_folder_name: Option<String>,
    /// Email of the connected Google account (display only).
    pub google_account_email: Option<String>,
    /// Hours to keep local audio after a successful save.
    pub audio_retention_hours: u32,
    /// Maximum recording length in seconds (failsafe).
    pub max_recording_secs: u32,
    /// Recordings shorter than this are discarded.
    pub min_recording_ms: u32,
    pub launch_at_login: bool,
    /// Set once the first-run setup has been completed.
    pub onboarding_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "control+alt+KeyR".into(),
            model: "base".into(),
            language: "ja".into(),
            whisper_cli_path: None,
            google_client_id: String::new(),
            google_client_secret: String::new(),
            google_project_number: None,
            drive_folder_id: None,
            drive_folder_name: None,
            google_account_email: None,
            audio_retention_hours: 24,
            max_recording_secs: 600,
            min_recording_ms: 300,
            launch_at_login: true,
            onboarding_done: false,
        }
    }
}

impl Settings {
    /// Load from disk; missing/corrupt file → defaults (corrupt file is backed up as `.bak`).
    /// Env vars `HOLDNOTE_GOOGLE_CLIENT_ID` / `HOLDNOTE_GOOGLE_CLIENT_SECRET` override empty values.
    pub fn load() -> Self {
        let path = crate::paths::settings_path();
        let mut s = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                log::warn!("settings parse error, using defaults: {e}");
                let _ = std::fs::rename(&path, path.with_extension("toml.bak"));
                Settings::default()
            }),
            Err(_) => Settings::default(),
        };
        if s.google_client_id.is_empty() {
            if let Ok(v) = std::env::var("HOLDNOTE_GOOGLE_CLIENT_ID") {
                s.google_client_id = v;
            }
        }
        if s.google_client_secret.is_empty() {
            if let Ok(v) = std::env::var("HOLDNOTE_GOOGLE_CLIENT_SECRET") {
                s.google_client_secret = v;
            }
        }
        s
    }

    /// Atomic save (write tmp + rename).
    pub fn save(&self) -> anyhow::Result<()> {
        let path = crate::paths::settings_path();
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn google_configured(&self) -> bool {
        !self.google_client_id.is_empty() && !self.google_client_secret.is_empty()
    }
}
