//! Application wiring: tao event loop (main thread), tray icon + menu, hotkey, recorder, overlay,
//! settings window, worker. OWNER: app agent.

use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use block2::RcBlock;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSWorkspace, NSWorkspaceWillSleepNotification};
use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSObjectProtocol, NSString};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::events::{AppEvent, EventSink, UiAction};
use crate::hotkey::{self, HotkeyController};
use crate::jobs::{Job, JobStatus, JobStore};
use crate::notifications::notify;
use crate::paths;
use crate::recorder::{MicPermission, Recorder, RecorderError, RecordingResult};
use crate::settings::Settings;
use crate::storage::drive_api::DriveApiBackend;
use crate::storage::oauth::{self, OAuthClient};
use crate::transcriber::{self, ModelStatus};
use crate::ui::overlay::{Overlay, OverlayState};
use crate::ui::settings_window::{SettingsViewModel, SettingsWindow};
use crate::worker::{self, WorkerHandle};

const ICON_1X: &[u8] = include_bytes!("../assets/menubar/holdnote-template.png");
const ICON_2X: &[u8] = include_bytes!("../assets/menubar/holdnote-template@2x.png");

const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Peak amplitude below which a recording is considered silent.
const SILENCE_PEAK: f32 = 0.01;

/// The tao user event type (private: `events.rs` is shared).
#[derive(Debug)]
enum LoopEvent {
    App(AppEvent),
    Menu(MenuEvent),
    Hotkey(GlobalHotKeyEvent),
    /// System sleep or screen lock (REC-08).
    SystemSleep,
}

struct ActiveRecording {
    job_id: String,
    started: Instant,
    /// Started from the tray menu (no hotkey failsafe).
    manual: bool,
    missed_held_polls: u32,
    /// The key-state query reported the hotkey as held at least once. Without Input Monitoring
    /// permission macOS answers "not pressed" for every key, so the failsafe only trusts the
    /// query after it has proven to work during this recording.
    seen_held: bool,
}

struct MenuItems {
    status: MenuItem,
    toggle: MenuItem,
    open_last: MenuItem,
    pending: MenuItem,
    history: MenuItem,
    retry_all: MenuItem,
    settings: MenuItem,
    about: MenuItem,
    quit: MenuItem,
}

struct TrayParts {
    // Kept alive for the lifetime of the app.
    tray: TrayIcon,
    items: MenuItems,
    /// Whether the colored recording icon is currently shown.
    recording_icon: bool,
}

struct App {
    mtm: MainThreadMarker,
    sink: EventSink,
    settings: Arc<RwLock<Settings>>,
    store: Arc<JobStore>,
    worker: WorkerHandle,
    recorder: Recorder,
    hotkey: Option<HotkeyController>,
    overlay: Overlay,
    settings_window: Option<SettingsWindow>,
    tray: Option<TrayParts>,
    rec: Option<ActiveRecording>,
    hotkey_down: bool,
    next_poll: Instant,
    last_status: HashMap<String, JobStatus>,
    // view-model state
    hotkey_error: Option<String>,
    drive_message: Option<(String, bool)>,
    google_connected: bool,
    model_progress: Option<(u64, u64)>,
    model_message: Option<String>,
    // kept alive so observers stay registered
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    started: bool,
    overlay_next: Option<Instant>,
    quit: bool,
}

pub fn run() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--overlay-demo") {
        let mtm = MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("must run on the main thread"))?;
        crate::ui::overlay::demo(mtm);
        return Ok(());
    }

    let mtm = MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("must run on the main thread"))?;
    let settings = Arc::new(RwLock::new(Settings::load()));
    let store = Arc::new(JobStore::open(&paths::db_path())?);

    let mut builder = EventLoopBuilder::<LoopEvent>::with_user_event();
    let mut event_loop = builder.build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let proxy = event_loop.create_proxy();

    let sink: EventSink = {
        let p = proxy.clone();
        Arc::new(move |e| {
            let _ = p.send_event(LoopEvent::App(e));
        })
    };
    {
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(LoopEvent::Menu(e));
        }));
        let p = proxy.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(LoopEvent::Hotkey(e));
        }));
    }

    let worker = worker::spawn(store.clone(), settings.clone(), sink.clone());
    let observers = install_sleep_observers(&proxy);

    let mut app = App {
        mtm,
        sink: sink.clone(),
        settings,
        store,
        worker,
        recorder: Recorder::new(sink.clone()),
        hotkey: None,
        overlay: Overlay::new(mtm),
        settings_window: None,
        tray: None,
        rec: None,
        hotkey_down: false,
        next_poll: Instant::now(),
        last_status: HashMap::new(),
        hotkey_error: None,
        drive_message: None,
        google_connected: false,
        model_progress: None,
        model_message: None,
        _observers: observers,
        started: false,
        overlay_next: None,
        quit: false,
    };

    event_loop.run(move |event, _target, control_flow| {
        match event {
            Event::NewEvents(StartCause::Init) => app.on_start(),
            Event::UserEvent(ev) => app.on_loop_event(ev),
            _ => {}
        }
        app.on_timers();
        *control_flow = app.next_control_flow();
    });
}

fn install_sleep_observers(
    proxy: &tao::event_loop::EventLoopProxy<LoopEvent>,
) -> Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>> {
    let mut out = Vec::new();
    // Sleep: NSWorkspaceWillSleepNotification.
    {
        let p = proxy.clone();
        let block = RcBlock::new(move |_n: NonNull<NSNotification>| {
            let _ = p.send_event(LoopEvent::SystemSleep);
        });
        unsafe {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            out.push(center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceWillSleepNotification),
                None,
                None,
                &block,
            ));
        }
    }
    // Screen lock: distributed notification.
    {
        let p = proxy.clone();
        let block = RcBlock::new(move |_n: NonNull<NSNotification>| {
            let _ = p.send_event(LoopEvent::SystemSleep);
        });
        let name = NSString::from_str("com.apple.screenIsLocked");
        unsafe {
            let center = NSDistributedNotificationCenter::defaultCenter();
            out.push(center.addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block));
        }
    }
    out
}

/// Menu bar icon. `tint` None → template (black, macOS adapts it to light/dark);
/// Some(color) → the silhouette filled with that solid color (used while recording).
fn load_icon(tint: Option<crate::ui::theme::Rgb>) -> Option<Icon> {
    for bytes in [ICON_2X, ICON_1X] {
        if let Ok(img) = image::load_from_memory(bytes) {
            let mut rgba = img.to_rgba8();
            if let Some(c) = tint {
                for px in rgba.pixels_mut() {
                    px.0[0] = c.0;
                    px.0[1] = c.1;
                    px.0[2] = c.2;
                }
            }
            let (w, h) = rgba.dimensions();
            if let Ok(icon) = Icon::from_rgba(rgba.into_raw(), w, h) {
                return Some(icon);
            }
        }
    }
    None
}

fn open_url(url: &str) {
    // Only open Google Drive/Docs links (webViewLink); never arbitrary URLs or file:// paths.
    let allowed = url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https" && matches!(u.host_str(), Some("drive.google.com" | "docs.google.com"))
    });
    if !allowed {
        log::warn!("refusing to open non-Drive url");
        return;
    }
    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
}

fn clear_dir_contents(dir: &std::path::Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
    }
}

impl App {
    // ---------------------------------------------------------------- lifecycle

    fn on_start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        // Leftover whisper output (may contain transcript text) from a previous crash.
        clear_dir_contents(&paths::tmp_dir());
        self.google_connected = oauth::is_connected();
        self.build_tray();

        match HotkeyController::new() {
            Ok(mut hk) => {
                let spec = self.settings.read().unwrap().hotkey.clone();
                if let Err(e) = hk.register(&spec) {
                    log::warn!("hotkey registration failed: {e}");
                    self.hotkey_error = Some(e.clone());
                    notify("HoldNote", &format!("ホットキーを登録できません: {e}。設定から変更してください。"));
                }
                self.hotkey = Some(hk);
            }
            Err(e) => {
                log::error!("hotkey controller: {e:#}");
                self.hotkey_error = Some(e.to_string());
            }
        }

        let onboarding_done = self.settings.read().unwrap().onboarding_done;
        if !onboarding_done {
            if crate::install::bundle_path().is_some() {
                let want = self.settings.read().unwrap().launch_at_login;
                if let Err(e) = crate::install::set_launch_at_login(want) {
                    log::warn!("launch at login: {e:#}");
                }
            }
            self.probe_mic_permission();
            let model = self.settings.read().unwrap().model.clone();
            if transcriber::model_status(&model, false) != ModelStatus::Present {
                notify("HoldNote", "音声認識モデルが未導入です。設定からダウンロードしてください。");
            }
            self.show_settings();
        }
        self.refresh_tray();
    }

    /// Triggers the macOS microphone prompt on first run by opening and immediately closing the input.
    fn probe_mic_permission(&mut self) {
        if crate::recorder::mic_permission() == MicPermission::NotDetermined {
            let probe = paths::audio_dir().join(".mic-probe.wav");
            if self.recorder.start(&probe).is_ok() {
                self.recorder.cancel();
            }
            let _ = std::fs::remove_file(&probe);
        }
    }

    fn next_control_flow(&self) -> ControlFlow {
        if self.quit {
            return ControlFlow::Exit;
        }
        let mut next: Option<Instant> = None;
        if self.rec.is_some() {
            next = Some(self.next_poll);
        }
        if let Some(t) = self.overlay_next {
            next = Some(next.map_or(t, |n| n.min(t)));
        }
        match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        }
    }


    fn on_timers(&mut self) {
        let now = Instant::now();
        if self.rec.is_some() && now >= self.next_poll {
            self.next_poll = now + POLL_INTERVAL;
            self.poll_recording();
        }
        self.overlay_next = self.overlay.tick();
    }

    // ---------------------------------------------------------------- events

    fn on_loop_event(&mut self, ev: LoopEvent) {
        match ev {
            LoopEvent::App(e) => self.on_app_event(e),
            LoopEvent::Menu(e) => self.on_menu_event(e),
            LoopEvent::Hotkey(e) => self.on_hotkey_event(e),
            LoopEvent::SystemSleep => {
                if self.rec.is_some() {
                    log::info!("system sleep / screen lock: stopping recording");
                    self.stop_recording();
                }
            }
        }
    }

    fn on_hotkey_event(&mut self, e: GlobalHotKeyEvent) {
        let is_ours = self.hotkey.as_ref().and_then(|h| h.current_id()) == Some(e.id);
        if !is_ours {
            return;
        }
        log::info!("hotkey {:?}", e.state);
        match e.state {
            HotKeyState::Pressed => {
                // REC-03: ignore autorepeat / double press.
                if self.hotkey_down || self.rec.is_some() {
                    return;
                }
                self.hotkey_down = true;
                self.start_recording(false);
            }
            HotKeyState::Released => {
                self.hotkey_down = false;
                if matches!(&self.rec, Some(r) if !r.manual) {
                    self.stop_recording();
                }
            }
        }
    }

    fn on_menu_event(&mut self, e: MenuEvent) {
        let Some(t) = &self.tray else { return };
        let id: &MenuId = &e.id;
        let it = &t.items;
        if id == it.toggle.id() {
            if self.rec.is_some() {
                self.stop_recording();
            } else {
                self.start_recording(true);
            }
        } else if id == it.open_last.id() {
            if let Ok(Some(job)) = self.store.last_saved() {
                if let Some(link) = job.remote_web_link {
                    open_url(&link);
                }
            }
        } else if id == it.history.id() || id == it.settings.id() {
            self.show_settings();
        } else if id == it.retry_all.id() {
            self.worker.retry_all();
        } else if id == it.about.id() {
            show_about();
        } else if id == it.quit.id() {
            if self.rec.is_some() {
                self.stop_recording();
            }
            self.quit = true;
        }
    }

    fn on_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::JobChanged { job_id, status, detail } => self.on_job_changed(job_id, status, detail),
            AppEvent::ModelDownload { model, downloaded, total, finished } => {
                match finished {
                    None => self.model_progress = Some((downloaded, total)),
                    Some(Ok(())) => {
                        self.model_progress = None;
                        self.model_message = None;
                        notify("HoldNote", &format!("音声認識モデル({model})の準備ができました"));
                        self.worker.retry_all();
                    }
                    Some(Err(e)) => {
                        self.model_progress = None;
                        self.model_message = Some(format!("ダウンロード失敗: {e}"));
                        notify("HoldNote", "音声認識モデルのダウンロードに失敗しました");
                    }
                }
                self.refresh_settings_window();
            }
            AppEvent::GoogleAuthFinished(res) => {
                match res {
                    Ok(email) => {
                        self.google_connected = true;
                        if !email.is_empty() {
                            self.update_settings(|s| s.google_account_email = Some(email));
                        }
                        self.drive_message = Some(("Googleアカウントに接続しました".into(), false));
                        self.worker.retry_all();
                    }
                    Err(e) => self.drive_message = Some((format!("Google連携に失敗しました: {e}"), true)),
                }
                self.refresh_settings_window();
            }
            AppEvent::FolderPicked(res) => {
                match res {
                    Ok(f) => {
                        self.google_connected = true;
                        let name = f.name.clone();
                        self.update_settings(|s| {
                            s.drive_folder_id = Some(f.id);
                            s.drive_folder_name = Some(f.name);
                        });
                        self.drive_message = Some((format!("保存先フォルダ: {name}"), false));
                        self.worker.retry_all();
                    }
                    Err(e) => self.drive_message = Some((format!("フォルダを選択できませんでした: {e}"), true)),
                }
                self.refresh_settings_window();
            }
            AppEvent::TestSaveFinished(res) => {
                match res {
                    Ok(()) => {
                        self.update_settings(|s| s.onboarding_done = true);
                        self.drive_message = Some(("保存テストに成功しました".into(), false));
                        notify("HoldNote", "保存テストに成功しました");
                    }
                    Err(e) => self.drive_message = Some((format!("保存テストに失敗しました: {e}"), true)),
                }
                self.refresh_settings_window();
            }
            AppEvent::RecorderFailed(msg) => {
                log::warn!("recorder failed: {msg}");
                if self.rec.is_some() {
                    self.stop_recording();
                }
                self.overlay.set_state(OverlayState::ActionRequired(format!("録音が中断されました: {msg}")));
                notify("HoldNote", "録音が中断されました。マイクの接続を確認してください。");
            }
            AppEvent::Ui(a) => {
                self.on_ui_action(a);
                self.refresh_settings_window();
            }
        }
    }

    fn on_job_changed(&mut self, job_id: String, status: JobStatus, detail: Option<String>) {
        if self.last_status.len() > 500 {
            self.last_status.clear();
        }
        let prev = self.last_status.insert(job_id.clone(), status);
        self.refresh_tray();
        self.refresh_settings_window();
        if prev == Some(status) {
            return;
        }

        if matches!(status, JobStatus::ActionRequired | JobStatus::Error) {
            let reason = detail.clone().unwrap_or_else(|| status.label_ja().to_string());
            notify("HoldNote", &format!("保存できませんでした: {reason}"));
        }

        // Only the most recent job drives the overlay, and never while recording.
        if self.rec.is_some() {
            return;
        }
        let is_latest = matches!(self.store.list_recent(1), Ok(v) if v.first().map(|j| j.job_id == job_id).unwrap_or(false));
        if !is_latest {
            return;
        }
        let state = match status {
            JobStatus::Transcribing => Some(OverlayState::Transcribing),
            JobStatus::Saving => Some(OverlayState::Saving),
            JobStatus::RemoteConfirmed => Some(OverlayState::SavedRemote),
            JobStatus::RetryWait => Some(OverlayState::RetryWait(detail.unwrap_or_else(|| "再試行を待っています".into()))),
            JobStatus::ActionRequired | JobStatus::Error => {
                Some(OverlayState::ActionRequired(detail.unwrap_or_else(|| status.label_ja().to_string())))
            }
            _ => None,
        };
        if let Some(s) = state {
            self.overlay.set_state(s);
        }
    }

    // ---------------------------------------------------------------- recording

    fn start_recording(&mut self, manual: bool) {
        if self.rec.is_some() || self.recorder.is_recording() {
            return;
        }
        if crate::recorder::mic_permission() == MicPermission::Denied {
            self.recording_start_failed(&RecorderError::PermissionDenied);
            return;
        }
        let job_id = uuid::Uuid::new_v4().simple().to_string();
        let audio_path = paths::audio_dir().join(format!("{job_id}.wav"));
        let job = {
            let s = self.settings.read().unwrap();
            Job {
                job_id: job_id.clone(),
                created_at: chrono::Local::now().fixed_offset(),
                duration_ms: 0,
                audio_path: audio_path.clone(),
                markdown_path: None,
                backend_type: "drive_api".into(),
                destination: s.drive_folder_id.clone(),
                destination_name: s.drive_folder_name.clone(),
                status: JobStatus::Recording,
                attempts: 0,
                last_error_code: None,
                last_error_message: None,
                next_retry_at: None,
                remote_file_id: None,
                remote_web_link: None,
                content_sha256: None,
                transcript_preview: None,
                file_name: None,
                updated_at: chrono::Utc::now(),
            }
        };
        if let Err(e) = self.store.insert(&job) {
            log::error!("job insert failed: {e:#}");
            self.overlay.set_state(OverlayState::ActionRequired("ジョブを作成できませんでした".into()));
            return;
        }
        if let Err(e) = self.recorder.start(&audio_path) {
            let _ = self.store.delete(&job_id);
            let _ = std::fs::remove_file(&audio_path);
            self.recording_start_failed(&e);
            return;
        }
        self.rec = Some(ActiveRecording { job_id, started: Instant::now(), manual, missed_held_polls: 0, seen_held: false });
        self.next_poll = Instant::now() + POLL_INTERVAL;
        self.overlay.set_state(OverlayState::Recording { elapsed: Duration::ZERO, level: 0.0 });
        self.refresh_tray();
    }

    fn recording_start_failed(&mut self, e: &RecorderError) {
        log::warn!("recording start failed: {e}");
        self.overlay.set_state(OverlayState::ActionRequired(e.to_string()));
        match e {
            RecorderError::PermissionDenied => notify(
                "HoldNote",
                "マイクへのアクセスが許可されていません。システム設定 > プライバシーとセキュリティ > マイク で HoldNote を許可してください。",
            ),
            RecorderError::NoDevice => notify("HoldNote", "マイクが見つかりません。入力デバイスを確認してください。"),
            _ => {}
        }
        self.refresh_tray();
    }

    fn poll_recording(&mut self) {
        let Some(rec) = self.rec.as_mut() else { return };
        let elapsed = self.recorder.elapsed();
        self.overlay.update_recording(elapsed, self.recorder.level());

        let max = Duration::from_secs(self.settings.read().unwrap().max_recording_secs as u64);
        if elapsed >= max {
            log::info!("max recording duration reached");
            self.stop_recording();
            self.overlay.set_state(OverlayState::Info("最大録音時間に達したため停止しました".into()));
            return;
        }
        if !rec.manual {
            // REC-04: failsafe if the Released event was lost or modifiers were released first.
            let held = self.hotkey.as_ref().and_then(|h| h.current()).map(|hk| hotkey::is_physically_held(&hk));
            match held {
                Some(true) => {
                    rec.seen_held = true;
                    rec.missed_held_polls = 0;
                }
                Some(false) if rec.seen_held => rec.missed_held_polls += 1,
                _ => rec.missed_held_polls = 0,
            }
            if rec.seen_held && rec.missed_held_polls >= 2 {
                log::info!("hotkey no longer held: stopping recording (failsafe)");
                self.hotkey_down = false;
                self.stop_recording();
            }
        }
    }

    fn stop_recording(&mut self) {
        if let Some(r) = &self.rec {
            log::info!("recording stopped after {} ms (manual={})", r.started.elapsed().as_millis(), r.manual);
        }
        let Some(rec) = self.rec.take() else { return };
        let res = self.recorder.stop();
        self.finalize_recording(&rec.job_id, res);
        self.refresh_tray();
    }

    fn finalize_recording(&mut self, job_id: &str, res: Result<RecordingResult, RecorderError>) {
        let Ok(Some(mut job)) = self.store.get(job_id) else {
            log::warn!("job row missing at finalize");
            return;
        };
        let discard = |this: &mut Self, mut job: Job, path: &std::path::Path, msg: &str| {
            let _ = std::fs::remove_file(path);
            job.status = JobStatus::Discarded;
            job.updated_at = chrono::Utc::now();
            if let Err(e) = this.store.update(&job) {
                log::warn!("job update failed: {e:#}");
            }
            this.overlay.set_state(OverlayState::Info(msg.into()));
        };
        match res {
            Ok(r) => {
                let min = Duration::from_millis(self.settings.read().unwrap().min_recording_ms as u64);
                if r.duration < min {
                    discard(self, job, &r.path, "録音が短すぎるため破棄しました");
                } else if r.peak == 0.0 {
                    // Digital silence: no real microphone delivers exact zeros, so the input is a
                    // virtual device (e.g. BlackHole) or access is blocked. Tell the user which one.
                    let input = crate::recorder::default_input_name().unwrap_or_else(|| "不明".into());
                    log::warn!("recording was digitally silent (input device: {input})");
                    discard(self, job, &r.path, "");
                    let msg = format!("音声が入力されていません（入力: {input}）");
                    self.overlay.set_state(OverlayState::ActionRequired(msg.clone()));
                    notify("HoldNote", &format!("{msg}。システム設定 > サウンド > 入力 でマイクを選んでください。"));
                } else if r.peak < SILENCE_PEAK {
                    discard(self, job, &r.path, "無音のため破棄しました");
                } else {
                    job.duration_ms = r.duration.as_millis() as i64;
                    job.audio_path = r.path;
                    job.status = JobStatus::Queued;
                    job.updated_at = chrono::Utc::now();
                    if let Err(e) = self.store.update(&job) {
                        log::error!("job update failed: {e:#}");
                    }
                    self.last_status.insert(job.job_id.clone(), JobStatus::Queued);
                    self.worker.wake();
                    self.overlay.set_state(OverlayState::Transcribing);
                }
            }
            Err(e) => {
                log::warn!("recorder stop failed: {e}");
                // Keep whatever audio exists so the user does not lose it.
                match crate::recorder::repair_wav(&job.audio_path) {
                    Ok(d) if d >= Duration::from_millis(300) => {
                        job.duration_ms = d.as_millis() as i64;
                        job.status = JobStatus::Queued;
                        job.updated_at = chrono::Utc::now();
                        let _ = self.store.update(&job);
                        self.worker.wake();
                        self.overlay.set_state(OverlayState::Transcribing);
                    }
                    _ => {
                        let _ = std::fs::remove_file(&job.audio_path);
                        let _ = self.store.delete(job_id);
                        self.overlay.set_state(OverlayState::ActionRequired(format!("録音を保存できませんでした: {e}")));
                    }
                }
            }
        }
    }

    fn cancel_recording(&mut self) {
        if let Some(rec) = self.rec.take() {
            self.recorder.cancel();
            let _ = self.store.delete(&rec.job_id);
            self.overlay.set_state(OverlayState::Hidden);
        }
    }

    // ---------------------------------------------------------------- settings

    fn update_settings(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.settings.write().unwrap();
        f(&mut s);
        if let Err(e) = s.save() {
            log::warn!("settings save failed: {e:#}");
        }
    }

    fn oauth_client(&self) -> OAuthClient {
        let s = self.settings.read().unwrap();
        OAuthClient {
            client_id: s.google_client_id.clone(),
            client_secret: s.google_client_secret.clone(),
            project_number: s.google_project_number.clone(),
        }
    }

    fn show_settings(&mut self) {
        if self.settings_window.is_none() {
            self.settings_window = Some(SettingsWindow::new(self.mtm, self.sink.clone()));
        }
        let vm = self.build_vm();
        if let Some(w) = self.settings_window.as_mut() {
            w.show();
            w.refresh(&vm);
        }
    }

    fn refresh_settings_window(&mut self) {
        if !matches!(&self.settings_window, Some(w) if w.is_visible()) {
            return;
        }
        let vm = self.build_vm();
        if let Some(w) = self.settings_window.as_mut() {
            w.refresh(&vm);
        }
    }

    fn build_vm(&self) -> SettingsViewModel {
        let s = self.settings.read().unwrap().clone();
        let model_status = if let Some((d, t)) = self.model_progress {
            let pct = if t > 0 { d * 100 / t } else { 0 };
            format!("ダウンロード中 {pct}%")
        } else {
            match transcriber::model_status(&s.model, false) {
                ModelStatus::Present => "導入済み".to_string(),
                ModelStatus::Missing => self.model_message.clone().unwrap_or_else(|| "未導入".into()),
                ModelStatus::Corrupt => "破損しています（再ダウンロードしてください）".to_string(),
            }
        };
        let permission = match crate::recorder::mic_permission() {
            MicPermission::Granted => "許可済み",
            MicPermission::Denied => "拒否されています（システム設定 > プライバシーとセキュリティ > マイク）",
            MicPermission::NotDetermined => "未確認（最初の録音時に許可を求めます）",
        };
        // Recordings use macOS's default input; show it so a virtual device (e.g. BlackHole) is obvious.
        let mic_status = match crate::recorder::default_input_name() {
            Some(name) => format!("{permission} ／ 入力: {name}（システム設定 > サウンド > 入力 で変更）"),
            None => format!("{permission} ／ 入力デバイスがありません"),
        };
        SettingsViewModel {
            hotkey_display: hotkey::display_string(&s.hotkey),
            hotkey_spec: s.hotkey.clone(),
            hotkey_error: self.hotkey_error.clone(),
            mic_status,
            model: s.model.clone(),
            model_status,
            whisper_cli: transcriber::find_whisper_cli(s.whisper_cli_path.as_deref())
                .map(|p| p.to_string_lossy().into_owned()),
            google_client_id: s.google_client_id.clone(),
            google_client_configured: s.google_configured(),
            google_connected: self.google_connected,
            google_account: s.google_account_email.clone(),
            drive_folder_name: s.drive_folder_name.clone(),
            drive_folder_id: s.drive_folder_id.clone(),
            drive_message: self.drive_message.clone(),
            audio_retention_hours: s.audio_retention_hours,
            launch_at_login: s.launch_at_login,
            pending_count: self.store.pending_count().unwrap_or(0),
            history: self.store.list_recent(100).unwrap_or_default(),
        }
    }

    fn on_ui_action(&mut self, a: UiAction) {
        match a {
            UiAction::ConnectGoogle => {
                if !self.settings.read().unwrap().google_configured() {
                    self.drive_message = Some(("先にGoogle OAuthのクライアントIDとシークレットを保存してください".into(), true));
                    return;
                }
                self.drive_message = Some(("ブラウザでGoogleにサインインしてください…".into(), false));
                let client = self.oauth_client();
                let sink = self.sink.clone();
                std::thread::spawn(move || {
                    let res = oauth::authorize_interactive(&client, false)
                        .map(|o| o.email.unwrap_or_default())
                        .map_err(|e| format!("{e:#}"));
                    sink(AppEvent::GoogleAuthFinished(res));
                });
            }
            UiAction::DisconnectGoogle => {
                match oauth::disconnect() {
                    Ok(()) => {
                        self.google_connected = false;
                        self.update_settings(|s| s.google_account_email = None);
                        self.drive_message = Some(("Google連携を解除しました".into(), false));
                    }
                    Err(e) => self.drive_message = Some((format!("連携を解除できませんでした: {e:#}"), true)),
                }
            }
            UiAction::PickDriveFolder => {
                if !self.settings.read().unwrap().google_configured() {
                    self.drive_message = Some(("先にGoogle OAuthのクライアントIDとシークレットを保存してください".into(), true));
                    return;
                }
                self.drive_message = Some(("ブラウザでフォルダを選択してください…".into(), false));
                let backend = DriveApiBackend::new(self.oauth_client());
                let sink = self.sink.clone();
                std::thread::spawn(move || {
                    let res = backend.pick_folder_interactive().map_err(|e| format!("{e:#}"));
                    sink(AppEvent::FolderPicked(res));
                });
            }
            UiAction::TestSave => {
                let folder = self.settings.read().unwrap().drive_folder_id.clone();
                let Some(folder) = folder else {
                    self.drive_message = Some(("先に保存先フォルダを選択してください".into(), true));
                    return;
                };
                self.drive_message = Some(("保存テスト中…".into(), false));
                let backend = DriveApiBackend::new(self.oauth_client());
                let sink = self.sink.clone();
                std::thread::spawn(move || {
                    let res = backend.test_save(&folder).map_err(|e| e.to_string());
                    sink(AppEvent::TestSaveFinished(res));
                });
            }
            UiAction::SetGoogleClient { client_id, client_secret } => {
                self.update_settings(|s| {
                    s.google_client_id = client_id.trim().to_string();
                    // An empty secret field means "keep the saved one" (the field is never pre-filled).
                    if !client_secret.trim().is_empty() {
                        s.google_client_secret = client_secret.trim().to_string();
                    }
                });
                self.drive_message = Some(("クライアント情報を保存しました".into(), false));
            }
            UiAction::SetHotkey(spec) => {
                let old = self.settings.read().unwrap().hotkey.clone();
                let Some(hk) = self.hotkey.as_mut() else {
                    self.hotkey_error = Some("ホットキーを初期化できていません".into());
                    return;
                };
                match hk.register(&spec) {
                    Ok(_) => {
                        self.hotkey_error = None;
                        self.hotkey_down = false;
                        self.update_settings(|s| s.hotkey = spec);
                    }
                    Err(e) => {
                        // Keep the previous hotkey working.
                        if hk.current().is_none() {
                            let _ = hk.register(&old);
                        }
                        self.hotkey_error = Some(e);
                    }
                }
            }
            UiAction::HotkeyCaptureBegan => {
                if self.rec.is_none() {
                    if let Some(hk) = self.hotkey.as_mut() {
                        hk.suspend();
                    }
                }
            }
            UiAction::HotkeyCaptureEnded => {
                let spec = self.settings.read().unwrap().hotkey.clone();
                if let Some(hk) = self.hotkey.as_mut() {
                    if let Err(e) = hk.resume(&spec) {
                        self.hotkey_error = Some(e);
                    }
                }
            }
            UiAction::SetModel(name) => {
                if transcriber::model_info(&name).is_some() {
                    self.update_settings(|s| s.model = name);
                    self.model_message = None;
                    self.worker.wake();
                }
            }
            UiAction::DownloadModel => {
                if self.model_progress.is_some() {
                    return;
                }
                let name = self.settings.read().unwrap().model.clone();
                self.model_progress = Some((0, 0));
                self.model_message = None;
                let sink = self.sink.clone();
                std::thread::spawn(move || {
                    let last_pct = AtomicU64::new(u64::MAX);
                    let progress_sink = sink.clone();
                    let n = name.clone();
                    let res = transcriber::download_model(&name, &move |d, t| {
                        let pct = if t > 0 { d * 100 / t } else { 0 };
                        if last_pct.swap(pct, Ordering::Relaxed) != pct {
                            progress_sink(AppEvent::ModelDownload {
                                model: n.clone(),
                                downloaded: d,
                                total: t,
                                finished: None,
                            });
                        }
                    });
                    sink(AppEvent::ModelDownload {
                        model: name,
                        downloaded: 0,
                        total: 0,
                        finished: Some(res.map(|_| ()).map_err(|e| format!("{e:#}"))),
                    });
                });
            }
            UiAction::SetWhisperCliPath(p) => {
                let p = p.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                self.update_settings(|s| s.whisper_cli_path = p);
            }
            UiAction::SetAudioRetentionHours(h) => self.update_settings(|s| s.audio_retention_hours = h),
            UiAction::SetLaunchAtLogin(on) => match crate::install::set_launch_at_login(on) {
                Ok(()) => self.update_settings(|s| s.launch_at_login = on),
                Err(e) => log::warn!("launch at login: {e:#}"),
            },
            UiAction::RetryJob(id) => self.worker.retry(&id),
            UiAction::RetryAll => self.worker.retry_all(),
            UiAction::DeleteJob(id) => {
                if self.rec.as_ref().map(|r| r.job_id == id).unwrap_or(false) {
                    return;
                }
                if let Ok(Some(job)) = self.store.get(&id) {
                    let _ = std::fs::remove_file(&job.audio_path);
                    if let Some(md) = &job.markdown_path {
                        let _ = std::fs::remove_file(md);
                    }
                }
                if let Err(e) = self.store.delete(&id) {
                    log::warn!("job delete failed: {e:#}");
                }
                self.last_status.remove(&id);
                self.refresh_tray();
            }
            UiAction::OpenJob(id) => {
                if let Ok(Some(job)) = self.store.get(&id) {
                    if let Some(link) = job.remote_web_link {
                        open_url(&link);
                    }
                }
            }
            UiAction::DeleteAllLocalData => {
                self.cancel_recording();
                clear_dir_contents(&paths::audio_dir());
                clear_dir_contents(&paths::markdown_dir());
                if let Err(e) = self.store.delete_all() {
                    log::warn!("delete_all failed: {e:#}");
                }
                self.last_status.clear();
                self.refresh_tray();
            }
            UiAction::StartTestRecording => {
                if self.rec.is_some() {
                    self.stop_recording();
                } else {
                    self.start_recording(true);
                }
            }
            UiAction::WindowClosed => {
                // The window may close while the shortcut field is still capturing.
                let spec = self.settings.read().unwrap().hotkey.clone();
                if let Some(hk) = self.hotkey.as_mut() {
                    let _ = hk.resume(&spec);
                }
                let (done, configured, model) = {
                    let s = self.settings.read().unwrap();
                    (
                        s.onboarding_done,
                        s.google_configured() && s.drive_folder_id.is_some(),
                        s.model.clone(),
                    )
                };
                if !done
                    && configured
                    && self.google_connected
                    && transcriber::model_status(&model, false) == ModelStatus::Present
                {
                    self.update_settings(|s| s.onboarding_done = true);
                }
            }
        }
    }

    // ---------------------------------------------------------------- tray

    fn build_tray(&mut self) {
        let status = MenuItem::new("状態: 待機", false, None);
        let toggle = MenuItem::new("録音開始", true, None);
        let open_last = MenuItem::new("最後のメモを開く", false, None);
        let pending = MenuItem::new("未保存: 0件", false, None);
        let history = MenuItem::new("履歴…", true, None);
        let retry_all = MenuItem::new("すべて再試行", true, None);
        let settings = MenuItem::new("設定…", true, None);
        let about = MenuItem::new("HoldNoteについて", true, None);
        let quit = MenuItem::new("終了", true, None);
        let menu = Menu::new();
        let r = menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &toggle,
            &open_last,
            &PredefinedMenuItem::separator(),
            &pending,
            &history,
            &retry_all,
            &PredefinedMenuItem::separator(),
            &settings,
            &about,
            &quit,
        ]);
        if let Err(e) = r {
            log::warn!("menu build: {e}");
        }
        let mut b = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("HoldNote");
        if let Some(icon) = load_icon(None) {
            b = b.with_icon_templated(icon);
        }
        match b.build() {
            Ok(tray) => {
                self.tray = Some(TrayParts {
                    recording_icon: false,
                    tray,
                    items: MenuItems { status, toggle, open_last, pending, history, retry_all, settings, about, quit },
                })
            }
            Err(e) => log::error!("tray icon: {e}"),
        }
    }

    fn refresh_tray(&mut self) {
        if self.tray.is_none() {
            return;
        }
        let pending = self.store.list_pending().unwrap_or_default();
        let (mut working, mut saving, mut waiting, mut action) = (0usize, 0usize, 0usize, 0usize);
        for j in &pending {
            match j.status {
                JobStatus::Recorded | JobStatus::Queued | JobStatus::Transcribing => working += 1,
                JobStatus::Transcribed | JobStatus::Saving => saving += 1,
                JobStatus::RetryWait => waiting += 1,
                JobStatus::ActionRequired | JobStatus::Error => {
                    action += 1;
                    waiting += 1;
                }
                _ => {}
            }
        }
        let unsaved = pending.iter().filter(|j| j.status != JobStatus::Recording).count();
        let (label, title): (String, Option<String>) = if self.rec.is_some() {
            ("状態: 録音中".into(), Some("● 録音中".into()))
        } else if working > 0 {
            ("状態: 文字起こし中".into(), Some("処理中".into()))
        } else if saving > 0 {
            ("状態: 保存中".into(), Some("処理中".into()))
        } else if action > 0 {
            (format!("状態: 要対応 ({action}件)"), Some(format!("保存待ち {waiting}")))
        } else if waiting > 0 {
            (format!("状態: 保存待ち ({waiting}件)"), Some(format!("保存待ち {waiting}")))
        } else {
            ("状態: 待機".into(), None)
        };
        let has_link = matches!(self.store.last_saved(), Ok(Some(j)) if j.remote_web_link.is_some());
        let recording = self.rec.is_some();

        let t = self.tray.as_mut().unwrap();
        if t.recording_icon != recording {
            let r = if recording {
                t.tray.set_icon_with_as_template(load_icon(Some(crate::ui::theme::RECORDING)), false)
            } else {
                t.tray.set_icon_templated(load_icon(None))
            };
            if let Err(e) = r {
                log::warn!("tray icon: {e}");
            }
            t.recording_icon = recording;
        }
        t.items.status.set_text(label);
        t.items.toggle.set_text(if recording { "録音停止" } else { "録音開始" });
        t.items.pending.set_text(format!("未保存: {unsaved}件"));
        t.items.open_last.set_enabled(has_link);
        t.items.retry_all.set_enabled(waiting > 0 || unsaved > 0);
        t.tray.set_title(title);
    }
}

fn show_about() {
    let msg = format!(
        "HoldNote v{}\\nホットキーを押している間だけ録音し、ローカルで文字起こしして Google Drive に保存します。",
        crate::notifications::escape_applescript(env!("CARGO_PKG_VERSION"))
    );
    let script = format!("display dialog \"{msg}\" with title \"HoldNote\" buttons {{\"OK\"}} default button 1");
    std::thread::spawn(move || {
        let _ = std::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).output();
    });
}
