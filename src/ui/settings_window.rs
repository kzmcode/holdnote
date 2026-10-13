//! Settings + history window (Coral Night, native AppKit, no WebView). Main thread only.
//! Sends user actions as `AppEvent::Ui(UiAction)` through the sink; the app pushes fresh data via `refresh`.
//! OWNER: settings-window agent.

use objc2::MainThreadMarker;

use crate::events::EventSink;
use crate::jobs::Job;

/// Everything the window displays. Built by the app.
#[derive(Debug, Clone, Default)]
pub struct SettingsViewModel {
    pub hotkey_spec: String,
    pub hotkey_display: String,
    /// Error from the last hotkey registration attempt.
    pub hotkey_error: Option<String>,
    pub mic_status: String,
    pub model: String,
    /// e.g. "導入済み", "未導入", "ダウンロード中 45%"
    pub model_status: String,
    pub whisper_cli: Option<String>,
    pub google_client_id: String,
    pub google_client_configured: bool,
    pub google_connected: bool,
    pub google_account: Option<String>,
    pub drive_folder_name: Option<String>,
    pub drive_folder_id: Option<String>,
    /// Status line for auth / picker / test save (text, is_error).
    pub drive_message: Option<(String, bool)>,
    pub audio_retention_hours: u32,
    pub launch_at_login: bool,
    pub pending_count: usize,
    pub history: Vec<Job>,
}

pub struct SettingsWindow {
    // implementation detail
}

impl SettingsWindow {
    pub fn new(mtm: MainThreadMarker, sink: EventSink) -> Self {
        let _ = (mtm, sink);
        todo!()
    }
    /// Show and bring to front (the app may temporarily activate itself for this window only).
    pub fn show(&mut self) {
        todo!()
    }
    pub fn is_visible(&self) -> bool {
        todo!()
    }
    /// Re-render with new data. Must be cheap enough to call on every job change.
    pub fn refresh(&mut self, vm: &SettingsViewModel) {
        let _ = vm;
        todo!()
    }
}
