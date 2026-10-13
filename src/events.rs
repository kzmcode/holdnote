//! Cross-thread events delivered to the main (UI) thread.
//! Background threads hold an `EventSink`; `app` forwards them into the tao event loop.

use std::sync::Arc;

use crate::jobs::JobStatus;
use crate::storage::PickedFolder;

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A job changed state (emitted by `worker`). `detail` is a short user-facing reason (Japanese).
    JobChanged { job_id: String, status: JobStatus, detail: Option<String> },
    /// Whisper model download progress; `finished` is Some when done (Ok or error message).
    ModelDownload { model: String, downloaded: u64, total: u64, finished: Option<Result<(), String>> },
    /// Result of the Google OAuth sign-in (Ok(email or "") / Err(message)).
    GoogleAuthFinished(Result<String, String>),
    /// Result of the Drive Picker folder selection.
    FolderPicked(Result<PickedFolder, String>),
    /// Result of the "保存テスト" (create + delete a test file in Drive).
    TestSaveFinished(Result<(), String>),
    /// The audio input stream failed (device removed etc.). Recording must be stopped safely.
    RecorderFailed(String),
    /// User action coming from the settings/history window.
    Ui(UiAction),
}

/// Actions the settings/history window asks the app to perform.
#[derive(Debug, Clone)]
pub enum UiAction {
    ConnectGoogle,
    DisconnectGoogle,
    PickDriveFolder,
    TestSave,
    /// Save Google OAuth client credentials entered in the settings UI.
    SetGoogleClient { client_id: String, client_secret: String },
    /// New hotkey string (global_hotkey syntax). App validates/registers it and reports back.
    SetHotkey(String),
    SetModel(String),
    DownloadModel,
    SetWhisperCliPath(Option<String>),
    SetAudioRetentionHours(u32),
    SetLaunchAtLogin(bool),
    RetryJob(String),
    RetryAll,
    DeleteJob(String),
    /// Open the saved Drive file (webViewLink) for a job.
    OpenJob(String),
    /// Delete ALL local audio/markdown/history (never touches Drive). Window already confirmed.
    DeleteAllLocalData,
    StartTestRecording,
    WindowClosed,
}

pub type EventSink = Arc<dyn Fn(AppEvent) + Send + Sync>;
