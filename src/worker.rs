//! Background pipeline: QUEUED → TRANSCRIBING → TRANSCRIBED → SAVING → RemoteConfirmed / RetryWait / ActionRequired.
//! OWNER: jobs/worker agent.
//!
//! Never log transcript text or tokens: only job ids, statuses and error messages from backends.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use sha2::{Digest, Sha256};

use crate::events::{AppEvent, EventSink};
use crate::jobs::{Job, JobStatus, JobStore};
use crate::markdown::{self, MarkdownMeta};
use crate::settings::Settings;
use crate::storage::drive_api::DriveApiBackend;
use crate::storage::oauth::OAuthClient;
use crate::storage::{SaveBackend, SaveError, SaveReceipt};
use crate::{paths, recorder, transcriber};

enum Command {
    Wake,
    Retry(String),
    RetryAll,
}

/// Handle to the single background worker thread. Cheap to clone.
#[derive(Clone)]
pub struct WorkerHandle {
    tx: Sender<Command>,
}

impl WorkerHandle {
    /// Process queue now (new job enqueued, network back, settings changed...).
    pub fn wake(&self) {
        let _ = self.tx.send(Command::Wake);
    }
    /// Reset a failed/waiting job to Queued (or to Transcribed when markdown exists) and process it.
    pub fn retry(&self, job_id: &str) {
        let _ = self.tx.send(Command::Retry(job_id.to_string()));
    }
    pub fn retry_all(&self) {
        let _ = self.tx.send(Command::RetryAll);
    }
}

/// Spawn the worker. On start it performs crash recovery:
/// - jobs left in Transcribing → Queued, Saving → Transcribed (re-check Drive by job id before re-upload),
/// - jobs left in Recording whose WAV is recoverable → Recorded/Queued (use `recorder::repair_wav`),
/// - orphan WAVs in `paths::audio_dir()` without a job row are logged (and imported as Recorded jobs),
/// - deletes local audio of RemoteConfirmed jobs older than `audio_retention_hours`.
pub fn spawn(store: Arc<JobStore>, settings: Arc<RwLock<Settings>>, sink: EventSink) -> WorkerHandle {
    let (tx, rx) = mpsc::channel();
    let worker = Worker { store, settings, sink };
    let spawned = std::thread::Builder::new().name("holdnote-worker".into()).spawn(move || worker.run(rx));
    if let Err(e) = spawned {
        log::error!("failed to spawn worker thread: {e}");
    }
    WorkerHandle { tx }
}

const CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);
const MAX_IDLE_WAIT: Duration = Duration::from_secs(300);
const BACKOFF_BASE_SECS: f64 = 30.0;
const BACKOFF_CAP_SECS: f64 = 30.0 * 60.0;

/// Retry delay after `attempts` failed save attempts (1 → 30s, 2 → 1m, 4m, ... cap 30 min).
/// `jitter` in [-1.0, 1.0] scales a ±10% spread.
pub fn backoff_delay(attempts: i64, jitter: f64) -> Duration {
    let exp = (attempts.max(1) - 1).min(16) as i32;
    let base = (BACKOFF_BASE_SECS * 2f64.powi(exp)).min(BACKOFF_CAP_SECS);
    let factor = 1.0 + 0.1 * jitter.clamp(-1.0, 1.0);
    Duration::from_secs_f64(base * factor)
}

fn random_jitter() -> f64 {
    match getrandom::u32() {
        Ok(v) => (v as f64 / u32::MAX as f64) * 2.0 - 1.0,
        Err(_) => 0.0,
    }
}

/// Collapse whitespace and keep the first 80 characters.
fn make_preview(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(80).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

struct Worker {
    store: Arc<JobStore>,
    settings: Arc<RwLock<Settings>>,
    sink: EventSink,
}

impl Worker {
    fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn run(self, rx: Receiver<Command>) {
        self.recover();
        self.cleanup();
        let mut last_cleanup = Instant::now();
        loop {
            while let Ok(cmd) = rx.try_recv() {
                self.handle(cmd);
            }
            if last_cleanup.elapsed() >= CLEANUP_INTERVAL {
                self.cleanup();
                last_cleanup = Instant::now();
            }
            if let Some(job) = self.next_due() {
                self.process(job);
                continue;
            }
            let until_cleanup = CLEANUP_INTERVAL.saturating_sub(last_cleanup.elapsed());
            let timeout = self.until_next_retry().unwrap_or(MAX_IDLE_WAIT).min(MAX_IDLE_WAIT).min(until_cleanup);
            match rx.recv_timeout(timeout) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn handle(&self, cmd: Command) {
        match cmd {
            Command::Wake => {}
            Command::Retry(id) => match self.store.get(&id) {
                Ok(Some(job)) => self.reset_for_retry(job),
                Ok(None) => {}
                Err(e) => log::warn!("retry lookup failed: {e:#}"),
            },
            Command::RetryAll => match self.store.list_pending() {
                Ok(jobs) => jobs.into_iter().for_each(|j| self.reset_for_retry(j)),
                Err(e) => log::warn!("retry_all failed: {e:#}"),
            },
        }
    }

    fn reset_for_retry(&self, mut job: Job) {
        if !matches!(job.status, JobStatus::Error | JobStatus::ActionRequired | JobStatus::RetryWait) {
            return;
        }
        let has_md = job.markdown_path.as_ref().is_some_and(|p| p.exists());
        // The snapshot is kept on retry, except when it is missing or no longer reachable:
        // an explicit retry then targets the folder currently chosen in settings (API-10).
        let dest_broken = job.destination.as_deref().is_none_or(str::is_empty)
            || matches!(job.last_error_code.as_deref(), Some("destination_unavailable" | "no_destination"));
        if dest_broken {
            let s = self.settings.read().unwrap();
            if s.drive_folder_id.is_some() {
                job.destination = s.drive_folder_id.clone();
                job.destination_name = s.drive_folder_name.clone();
            }
        }
        job.next_retry_at = None;
        job.last_error_code = None;
        job.last_error_message = None;
        let status = if has_md { JobStatus::Transcribed } else { JobStatus::Queued };
        self.set_status(&mut job, status, None);
    }

    /// Persist the new status and notify the UI.
    fn set_status(&self, job: &mut Job, status: JobStatus, detail: Option<String>) {
        job.status = status;
        job.updated_at = Utc::now();
        if let Err(e) = self.store.update(job) {
            log::warn!("job {} update failed: {e:#}", job.job_id);
        }
        (self.sink)(AppEvent::JobChanged { job_id: job.job_id.clone(), status, detail });
    }

    fn fail(&self, job: &mut Job, status: JobStatus, code: &str, message: &str) {
        job.last_error_code = Some(code.to_string());
        job.last_error_message = Some(message.to_string());
        self.set_status(job, status, Some(message.to_string()));
    }

    fn next_due(&self) -> Option<Job> {
        let now = Utc::now();
        self.store.list_pending().ok()?.into_iter().find(|j| match j.status {
            JobStatus::Recorded | JobStatus::Queued | JobStatus::Transcribed | JobStatus::Saving => true,
            JobStatus::RetryWait => j.next_retry_at.is_none_or(|t| t <= now),
            _ => false,
        })
    }

    fn until_next_retry(&self) -> Option<Duration> {
        let now = Utc::now();
        self.store
            .list_pending()
            .ok()?
            .into_iter()
            .filter(|j| j.status == JobStatus::RetryWait)
            .filter_map(|j| j.next_retry_at)
            .min()
            .map(|t| (t - now).to_std().unwrap_or(Duration::ZERO))
    }

    fn process(&self, mut job: Job) {
        if matches!(job.status, JobStatus::Recorded | JobStatus::Queued) {
            if !self.transcribe_stage(&mut job) {
                return;
            }
        }
        self.save_stage(&mut job);
    }

    /// Returns true when the job reached Transcribed and may continue to saving.
    fn transcribe_stage(&self, job: &mut Job) -> bool {
        let s = self.settings();
        if !job.audio_path.exists() {
            self.fail(job, JobStatus::Error, "audio_missing", "音声ファイルが見つかりません");
            return false;
        }
        let Some(cli) = transcriber::find_whisper_cli(s.whisper_cli_path.as_deref()) else {
            self.fail(job, JobStatus::ActionRequired, "cli_missing", "whisper-cli が見つかりません");
            return false;
        };
        if transcriber::model_status(&s.model, false) != transcriber::ModelStatus::Present {
            self.fail(job, JobStatus::ActionRequired, "model_missing", "音声認識モデルがありません");
            return false;
        }
        let model = transcriber::model_path(&s.model);
        self.set_status(job, JobStatus::Transcribing, None);

        let secs = (job.duration_ms.max(0) as u64 / 1000) * 3 + 30;
        let timeout = Duration::from_secs(secs.max(60));
        match transcriber::transcribe(&cli, &model, &job.audio_path, &s.language, timeout) {
            Ok(text) => {
                let text = text.trim();
                if text.is_empty() {
                    self.set_status(job, JobStatus::Discarded, Some("音声が認識できませんでした".into()));
                    return false;
                }
                match self.write_markdown(job, &s, text) {
                    Ok(()) => {
                        job.last_error_code = None;
                        job.last_error_message = None;
                        self.set_status(job, JobStatus::Transcribed, None);
                        true
                    }
                    Err(e) => {
                        let msg = format!("ファイルを書き込めません: {e}");
                        self.fail(job, JobStatus::Error, "write_failed", &msg);
                        false
                    }
                }
            }
            Err(transcriber::TranscribeError::Empty) => {
                self.set_status(job, JobStatus::Discarded, Some("音声が認識できませんでした".into()));
                false
            }
            Err(transcriber::TranscribeError::CliNotFound) => {
                self.fail(job, JobStatus::ActionRequired, "cli_missing", "whisper-cli が見つかりません");
                false
            }
            Err(transcriber::TranscribeError::ModelMissing) => {
                self.fail(job, JobStatus::ActionRequired, "model_missing", "音声認識モデルがありません");
                false
            }
            Err(e) => {
                self.fail(job, JobStatus::Error, "transcribe_failed", &e.to_string());
                false
            }
        }
    }

    fn write_markdown(&self, job: &mut Job, s: &Settings, text: &str) -> std::io::Result<()> {
        let meta = MarkdownMeta {
            created_at: job.created_at,
            duration_seconds: job.duration_ms.max(0) as f64 / 1000.0,
            language: s.language.clone(),
            save_mode: "drive_api".into(),
        };
        let content = markdown::render(&meta, text);
        let name = job.file_name.clone().unwrap_or_else(|| markdown::file_name(&job.created_at, &job.job_id));
        let path: PathBuf = paths::markdown_dir().join(&name);
        write_atomic(&path, content.as_bytes())?;
        job.file_name = Some(name);
        job.markdown_path = Some(path);
        job.content_sha256 = Some(sha256_hex(content.as_bytes()));
        job.transcript_preview = Some(make_preview(text));
        Ok(())
    }

    fn save_stage(&self, job: &mut Job) {
        if job.destination.as_deref().is_none_or(str::is_empty) {
            self.fail(job, JobStatus::ActionRequired, "no_destination", "保存先フォルダが未設定です");
            return;
        }
        let s = self.settings();
        if !s.google_configured() {
            self.fail(job, JobStatus::ActionRequired, "client_missing", "Googleのクライアント設定がありません");
            return;
        }
        let md = match job.markdown_path.as_ref().map(std::fs::read_to_string) {
            Some(Ok(md)) => md,
            _ => {
                // Markdown lost: regenerate from audio if we still have it.
                if job.audio_path.exists() {
                    job.markdown_path = None;
                    self.set_status(job, JobStatus::Queued, None);
                } else {
                    self.fail(job, JobStatus::Error, "markdown_missing", "保存用ファイルが見つかりません");
                }
                return;
            }
        };

        let previous_attempts = job.attempts;
        job.attempts += 1;
        job.next_retry_at = None;
        self.set_status(job, JobStatus::Saving, None);

        let backend = DriveApiBackend::new(OAuthClient {
            client_id: s.google_client_id.clone(),
            client_secret: s.google_client_secret.clone(),
            project_number: s.google_project_number.clone(),
        });
        // The backend checks for an existing remote file when attempts > 0, so pass the
        // count from before this attempt unless a crash left an attempt in flight.
        let mut for_backend = job.clone();
        for_backend.attempts = previous_attempts;
        match backend.save(&for_backend, &md) {
            Ok(SaveReceipt::RemoteConfirmed { file_id, web_view_link }) => {
                job.remote_file_id = Some(file_id);
                job.remote_web_link = web_view_link;
                job.last_error_code = None;
                job.last_error_message = None;
                self.set_status(job, JobStatus::RemoteConfirmed, None);
            }
            Err(e) => self.handle_save_error(job, e),
        }
    }

    fn handle_save_error(&self, job: &mut Job, e: SaveError) {
        let msg = e.to_string();
        match e {
            SaveError::Retryable(_) => {
                let delay = backoff_delay(job.attempts, random_jitter());
                job.next_retry_at = Some(Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default());
                self.fail(job, JobStatus::RetryWait, e.code(), &msg);
            }
            _ => self.fail(job, JobStatus::ActionRequired, e.code(), &msg),
        }
    }

    // ---- startup recovery / retention ----

    fn recover(&self) {
        let jobs = match self.store.list_pending() {
            Ok(j) => j,
            Err(e) => {
                log::warn!("recovery: cannot list jobs: {e:#}");
                return;
            }
        };
        for mut job in jobs {
            match job.status {
                JobStatus::Transcribing => self.set_status(&mut job, JobStatus::Queued, None),
                JobStatus::Saving => self.set_status(&mut job, JobStatus::Transcribed, None),
                JobStatus::Recording => match recorder::repair_wav(&job.audio_path) {
                    Ok(d) => {
                        job.duration_ms = d.as_millis() as i64;
                        self.set_status(&mut job, JobStatus::Recorded, Some("録音を復元しました".into()));
                    }
                    Err(e) => {
                        log::warn!("recovery: job {} wav unrecoverable: {e:#}", job.job_id);
                        self.fail(&mut job, JobStatus::Error, "wav_unrecoverable", "録音ファイルを復元できませんでした");
                    }
                },
                _ => {}
            }
        }
        self.import_orphans();
    }

    fn import_orphans(&self) {
        let Ok(rd) = std::fs::read_dir(paths::audio_dir()) else { return };
        let s = self.settings();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("wav") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
            if matches!(self.store.get(&stem), Ok(Some(_))) {
                continue;
            }
            log::warn!("orphan audio file without job row: {stem}");
            let Ok(dur) = recorder::repair_wav(&path) else { continue };
            let created = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(DateTime::<Local>::from)
                .unwrap_or_else(|_| Local::now());
            let job = Job {
                job_id: stem.clone(),
                created_at: created.fixed_offset(),
                duration_ms: dur.as_millis() as i64,
                audio_path: path,
                markdown_path: None,
                backend_type: "drive_api".into(),
                destination: s.drive_folder_id.clone(),
                destination_name: s.drive_folder_name.clone(),
                status: JobStatus::Recorded,
                attempts: 0,
                last_error_code: None,
                last_error_message: None,
                next_retry_at: None,
                remote_file_id: None,
                remote_web_link: None,
                content_sha256: None,
                transcript_preview: None,
                file_name: None,
                updated_at: Utc::now(),
            };
            match self.store.insert(&job) {
                Ok(()) => (self.sink)(AppEvent::JobChanged {
                    job_id: stem,
                    status: JobStatus::Recorded,
                    detail: Some("残っていた録音を取り込みました".into()),
                }),
                Err(e) => log::warn!("orphan import failed: {e:#}"),
            }
        }
    }

    /// Delete local audio of saved (and discarded) jobs past the retention period. Markdown is kept.
    fn cleanup(&self) {
        let hours = self.settings().audio_retention_hours as i64;
        let cutoff = Utc::now() - chrono::Duration::hours(hours);
        match self.store.list_confirmed_before(cutoff) {
            Ok(jobs) => {
                for j in jobs {
                    remove_file_quiet(&j.audio_path);
                }
            }
            Err(e) => log::warn!("retention cleanup failed: {e:#}"),
        }
    }
}

fn remove_file_quiet(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log::warn!("cannot remove {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_progression_and_cap() {
        let secs = |n| backoff_delay(n, 0.0).as_secs();
        assert_eq!(secs(1), 30);
        assert_eq!(secs(2), 60);
        assert_eq!(secs(3), 120);
        assert_eq!(secs(4), 240);
        assert_eq!(secs(7), 1800);
        assert_eq!(secs(100), 1800);
        assert_eq!(secs(0), 30);
    }

    #[test]
    fn backoff_jitter_range() {
        assert_eq!(backoff_delay(1, 1.0).as_secs_f64().round() as u64, 33);
        assert_eq!(backoff_delay(1, -1.0).as_secs_f64().round() as u64, 27);
        assert_eq!(backoff_delay(1, 5.0), backoff_delay(1, 1.0));
        let j = random_jitter();
        assert!((-1.0..=1.0).contains(&j));
    }

    #[test]
    fn preview_and_hash() {
        assert_eq!(make_preview("a\n b   c"), "a b c");
        assert_eq!(make_preview(&"あ".repeat(200)).chars().count(), 80);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
