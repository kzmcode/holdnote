//! Settings + history window (Coral Night, native AppKit, no WebView). Main thread only.
//! Sends user actions as `AppEvent::Ui(UiAction)` through the sink; the app pushes fresh data via `refresh`.
//! OWNER: settings-window agent.

use std::cell::RefCell;
use std::collections::HashMap;

use chrono::{DateTime, Local, TimeZone};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertStyle, NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSBackingStoreType, NSButton,
    NSControlStateValueOn, NSControlTextEditingDelegate, NSFocusRingType, NSFont, NSForegroundColorAttributeName, NSImage, NSImageView,
    NSLineBreakMode, NSOpenPanel, NSPopUpButton, NSScrollView, NSSecureTextField, NSSegmentStyle,
    NSSegmentSwitchTracking, NSSegmentedControl, NSStepper, NSTextField, NSTextFieldDelegate, NSView, NSWindow,
    NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSPoint,
    NSRect, NSSize, NSString,
};

use crate::events::{AppEvent, EventSink, UiAction};
use crate::jobs::{Job, JobStatus};
use crate::ui::theme::{self, ns_color, Rgb};

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

// ---------------------------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------------------------

/// "MM/dd HH:mm" in the timezone of `dt`.
fn format_md_hm<Tz: TimeZone>(dt: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    dt.format("%m/%d %H:%M").to_string()
}

fn format_local(dt: &DateTime<chrono::FixedOffset>) -> String {
    format_md_hm(&dt.with_timezone(&Local))
}

/// Single-line truncation by chars with an ellipsis; newlines collapse to spaces.
fn truncate_chars(s: &str, max: usize) -> String {
    let flat: String = s.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    let flat = flat.trim();
    if flat.chars().count() <= max {
        flat.to_string()
    } else {
        let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// (label, SF Symbol, color) for a job status; wording follows requirements §4.5.
fn status_info(s: JobStatus) -> (&'static str, &'static str, Rgb) {
    match s {
        JobStatus::Recording => ("録音中", "mic.fill", theme::RECORDING),
        JobStatus::Recorded => ("録音済み", "arrow.triangle.2.circlepath", theme::ACCENT),
        JobStatus::Queued => ("文字起こし待ち", "arrow.triangle.2.circlepath", theme::ACCENT),
        JobStatus::Transcribing => ("文字起こし中…", "arrow.triangle.2.circlepath", theme::ACCENT),
        JobStatus::Transcribed => ("文字起こし済み", "arrow.triangle.2.circlepath", theme::ACCENT),
        JobStatus::Saving => ("保存中…", "arrow.triangle.2.circlepath", theme::ACCENT),
        JobStatus::RemoteConfirmed => ("Google Driveに保存しました", "checkmark.circle.fill", theme::SUCCESS),
        JobStatus::RetryWait => ("保存待ち", "clock.fill", theme::WARNING),
        JobStatus::ActionRequired => ("保存できませんでした", "exclamationmark.triangle.fill", theme::ERROR),
        JobStatus::Error => ("文字起こしに失敗しました", "exclamationmark.triangle.fill", theme::ERROR),
        JobStatus::Discarded => ("破棄しました", "minus.circle", theme::TEXT_SECONDARY),
    }
}

fn can_retry(s: JobStatus) -> bool {
    matches!(s, JobStatus::RetryWait | JobStatus::ActionRequired | JobStatus::Error)
}

// ---------------------------------------------------------------------------------------------
// AppKit plumbing
// ---------------------------------------------------------------------------------------------

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "HNFlippedView"]
    struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

fn set_bg(view: &NSView, color: Rgb, border: Option<Rgb>, radius: f64) {
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setBackgroundColor(Some(&ns_color(color).CGColor()));
        layer.setCornerRadius(radius);
        if let Some(b) = border {
            layer.setBorderWidth(1.0);
            layer.setBorderColor(Some(&ns_color(b).CGColor()));
        }
    }
}

fn set_field_border(n: &NSNotification, color: Rgb) {
    let Some(obj) = n.object() else { return };
    if let Some(view) = obj.downcast_ref::<NSView>() {
        if let Some(layer) = view.layer() {
            layer.setBorderColor(Some(&ns_color(color).CGColor()));
        }
    }
}

fn card(mtm: MainThreadMarker, parent: &NSView, x: f64, y: f64, w: f64, h: f64) -> Retained<FlippedView> {
    let v = FlippedView::new(mtm, rect(x, y, w, h));
    set_bg(&v, theme::SURFACE, Some(theme::BORDER), 12.0);
    parent.addSubview(&v);
    v
}

#[allow(clippy::too_many_arguments)]
fn label(
    mtm: MainThreadMarker,
    parent: &NSView,
    text: &str,
    frame: NSRect,
    size: f64,
    bold: bool,
    color: Rgb,
    wrap: bool,
) -> Retained<NSTextField> {
    let t = ns(text);
    let l = if wrap {
        NSTextField::wrappingLabelWithString(&t, mtm)
    } else {
        NSTextField::labelWithString(&t, mtm)
    };
    l.setFrame(frame);
    let font = if bold { NSFont::boldSystemFontOfSize(size) } else { NSFont::systemFontOfSize(size) };
    l.setFont(Some(&font));
    l.setTextColor(Some(&ns_color(color)));
    if !wrap {
        l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    }
    parent.addSubview(&l);
    l
}

fn title(mtm: MainThreadMarker, parent: &NSView, text: &str) {
    label(mtm, parent, text, rect(16.0, 12.0, 400.0, 20.0), 14.0, true, theme::TEXT_PRIMARY, false);
}

fn text_field(
    mtm: MainThreadMarker,
    parent: &NSView,
    frame: NSRect,
    secure: bool,
    placeholder: &str,
) -> Retained<NSTextField> {
    let f: Retained<NSTextField> = if secure {
        let s = NSSecureTextField::initWithFrame(mtm.alloc(), frame);
        Retained::into_super(s)
    } else {
        NSTextField::initWithFrame(mtm.alloc(), frame)
    };
    f.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    f.setTextColor(Some(&ns_color(theme::TEXT_PRIMARY)));
    // Flat field: own 1pt border instead of the system bezel/focus ring (which uses the
    // user's system accent color). Editing switches the border to ACCENT (see Handler).
    f.setBezeled(false);
    f.setBordered(false);
    f.setFocusRingType(NSFocusRingType::None);
    f.setDrawsBackground(true);
    f.setBackgroundColor(Some(&ns_color(theme::BACKGROUND)));
    set_bg(&f, theme::BACKGROUND, Some(theme::BORDER), 6.0);
    f.setPlaceholderString(Some(&ns(placeholder)));
    parent.addSubview(&f);
    f
}

fn colored_title(text: &str, color: Rgb) -> Retained<NSAttributedString> {
    unsafe {
        let key: &NSString = NSForegroundColorAttributeName;
        let val: Retained<AnyObject> = Retained::cast_unchecked(ns_color(color));
        let dict = NSDictionary::from_retained_objects(&[key], &[val]);
        NSAttributedString::initWithString_attributes(NSAttributedString::alloc(), &ns(text), Some(&dict))
    }
}

#[derive(Clone, Copy)]
enum Style {
    Primary,
    Danger,
    Neutral,
}

fn style_button(b: &NSButton, text: &str, style: Style) {
    let bg = match style {
        Style::Primary => theme::ACCENT,
        Style::Danger => theme::RECORDING,
        Style::Neutral => theme::BORDER,
    };
    b.setBordered(false);
    b.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    b.setAttributedTitle(&colored_title(text, theme::TEXT_PRIMARY));
    set_bg(b, bg, None, 6.0);
}

fn set_enabled(b: &NSButton, enabled: bool) {
    b.setEnabled(enabled);
    b.setAlphaValue(if enabled { 1.0 } else { 0.4 });
}

#[derive(Clone)]
enum Act {
    Do(UiAction),
    Confirm { message: String, info: String, ok: String, action: UiAction },
    Hotkey,
    SaveClient,
    Retention,
    Launch,
    Model,
    PickCli,
}

struct Fields {
    hotkey: Retained<NSTextField>,
    client_id: Retained<NSTextField>,
    secret: Retained<NSTextField>,
    stepper: Retained<NSStepper>,
    popup: Retained<NSPopUpButton>,
}

struct HIvars {
    sink: EventSink,
    acts: RefCell<HashMap<isize, Act>>,
    next_tag: RefCell<isize>,
    fields: RefCell<Option<Fields>>,
    pages: RefCell<Vec<Retained<NSView>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HNSettingsHandler"]
    #[ivars = HIvars]
    struct Handler;

    unsafe impl NSObjectProtocol for Handler {}

    unsafe impl NSControlTextEditingDelegate for Handler {
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn text_did_begin(&self, n: &NSNotification) {
            set_field_border(n, theme::ACCENT);
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn text_did_end(&self, n: &NSNotification) {
            set_field_border(n, theme::BORDER);
        }
    }

    unsafe impl NSTextFieldDelegate for Handler {}

    unsafe impl NSWindowDelegate for Handler {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            (self.ivars().sink)(AppEvent::Ui(UiAction::WindowClosed));
        }
    }

    impl Handler {
        #[unsafe(method(clicked:))]
        fn clicked(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let act = self.ivars().acts.borrow().get(&tag).cloned();
            let Some(act) = act else { return };
            self.perform(act, sender);
        }

        #[unsafe(method(tabChanged:))]
        fn tab_changed(&self, sender: &AnyObject) {
            let idx: isize = unsafe { msg_send![sender, selectedSegment] };
            for (i, p) in self.ivars().pages.borrow().iter().enumerate() {
                p.setHidden(i as isize != idx);
            }
        }
    }
);

impl Handler {
    fn new(mtm: MainThreadMarker, sink: EventSink) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(HIvars {
            sink,
            acts: RefCell::new(HashMap::new()),
            next_tag: RefCell::new(1),
            fields: RefCell::new(None),
            pages: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn emit(&self, a: UiAction) {
        (self.ivars().sink)(AppEvent::Ui(a));
    }

    /// Registers an action and returns its tag (history-row tags are removed on rebuild).
    fn register(&self, act: Act) -> isize {
        let mut n = self.ivars().next_tag.borrow_mut();
        let tag = *n;
        *n += 1;
        self.ivars().acts.borrow_mut().insert(tag, act);
        tag
    }

    fn button(&self, parent: &NSView, text: &str, frame: NSRect, style: Style, act: Act) -> Retained<NSButton> {
        let mtm = MainThreadMarker::from(self);
        let b = unsafe { NSButton::buttonWithTitle_target_action(&ns(text), Some(self), Some(sel!(clicked:)), mtm) };
        b.setFrame(frame);
        b.setTag(self.register(act));
        style_button(&b, text, style);
        parent.addSubview(&b);
        b
    }

    fn confirm(&self, message: &str, info: &str, ok: &str) -> bool {
        let mtm = MainThreadMarker::from(self);
        let alert = NSAlert::new(mtm);
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.setMessageText(&ns(message));
        alert.setInformativeText(&ns(info));
        alert.addButtonWithTitle(&ns(ok));
        alert.addButtonWithTitle(&ns("キャンセル"));
        alert.runModal() == 1000
    }

    fn perform(&self, act: Act, sender: &AnyObject) {
        match act {
            Act::Do(a) => self.emit(a),
            Act::Confirm { message, info, ok, action } => {
                if self.confirm(&message, &info, &ok) {
                    self.emit(action);
                }
            }
            Act::Hotkey => {
                let spec = self.ivars().fields.borrow().as_ref().map(|f| f.hotkey.stringValue().to_string());
                if let Some(s) = spec {
                    let s = s.trim().to_string();
                    if !s.is_empty() {
                        self.emit(UiAction::SetHotkey(s));
                    }
                }
            }
            Act::SaveClient => {
                let v = self
                    .ivars()
                    .fields
                    .borrow()
                    .as_ref()
                    .map(|f| (f.client_id.stringValue().to_string(), f.secret.stringValue().to_string()));
                if let Some((id, secret)) = v {
                    self.emit(UiAction::SetGoogleClient {
                        client_id: id.trim().to_string(),
                        client_secret: secret.trim().to_string(),
                    });
                }
            }
            Act::Retention => {
                let v: f64 = unsafe { msg_send![sender, doubleValue] };
                self.emit(UiAction::SetAudioRetentionHours(v.max(0.0) as u32));
            }
            Act::Launch => {
                let st: isize = unsafe { msg_send![sender, state] };
                self.emit(UiAction::SetLaunchAtLogin(st == NSControlStateValueOn));
            }
            Act::Model => {
                let t: Option<Retained<NSString>> = unsafe { msg_send![sender, titleOfSelectedItem] };
                if let Some(t) = t {
                    self.emit(UiAction::SetModel(t.to_string()));
                }
            }
            Act::PickCli => {
                let mtm = MainThreadMarker::from(self);
                let panel = NSOpenPanel::openPanel(mtm);
                panel.setCanChooseFiles(true);
                panel.setCanChooseDirectories(false);
                panel.setAllowsMultipleSelection(false);
                panel.setMessage(Some(&ns("whisper-cli の実行ファイルを選択")));
                if panel.runModal() == 1 {
                    if let Some(p) = panel.URL().and_then(|u| u.path()) {
                        self.emit(UiAction::SetWhisperCliPath(Some(p.to_string())));
                    }
                }
            }
        }
    }
}

struct Widgets {
    hotkey_display: Retained<NSTextField>,
    hotkey_error: Retained<NSTextField>,
    mic: Retained<NSTextField>,
    launch: Retained<NSButton>,
    retention_label: Retained<NSTextField>,
    status: Retained<NSTextField>,
    connect: Retained<NSButton>,
    disconnect: Retained<NSButton>,
    folder: Retained<NSTextField>,
    pick: Retained<NSButton>,
    test_save: Retained<NSButton>,
    message: Retained<NSTextField>,
    model_status: Retained<NSTextField>,
    download: Retained<NSButton>,
    cli: Retained<NSTextField>,
    pending: Retained<NSTextField>,
    retry_all: Retained<NSButton>,
    history_doc: Retained<FlippedView>,
}

type Fingerprint = Vec<(String, JobStatus, i64, bool, Option<String>)>;

pub struct SettingsWindow {
    mtm: MainThreadMarker,
    window: Retained<NSWindow>,
    handler: Retained<Handler>,
    w: Widgets,
    history_fp: Option<Fingerprint>,
    history_tags: Vec<isize>,
}

const PAGE_W: f64 = 600.0;

impl SettingsWindow {
    pub fn new(mtm: MainThreadMarker, sink: EventSink) -> Self {
        let handler = Handler::new(mtm, sink);
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                rect(0.0, 0.0, 640.0, 600.0),
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&ns("HoldNote 設定"));
        // Flat title bar in the window background color (no system material).
        window.setTitlebarAppearsTransparent(true);
        unsafe {
            if let Some(a) = NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua) {
                window.setAppearance(Some(&a));
            }
        }
        window.setBackgroundColor(Some(&ns_color(theme::BACKGROUND)));
        window.setDelegate(Some(ProtocolObject::from_ref(&*handler)));
        window.center();

        let root = FlippedView::new(mtm, rect(0.0, 0.0, 640.0, 600.0));
        set_bg(&root, theme::BACKGROUND, None, 0.0);
        window.setContentView(Some(&root));

        // Tabs.
        let labels = NSArray::from_retained_slice(&[ns("一般"), ns("Google Drive"), ns("音声認識"), ns("履歴")]);
        let seg = unsafe {
            NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(
                &labels,
                NSSegmentSwitchTracking::SelectOne,
                Some(&*handler),
                Some(sel!(tabChanged:)),
                mtm,
            )
        };
        seg.setSegmentStyle(NSSegmentStyle::Automatic);
        seg.setSelectedSegmentBezelColor(Some(&theme::ns_color(theme::ACCENT)));
        seg.setSelectedSegment(0);
        seg.setFrame(rect(20.0, 14.0, 600.0, 28.0));
        seg.setAccessibilityLabel(Some(&ns("設定セクション")));
        root.addSubview(&seg);

        let mk_page = || {
            let p = FlippedView::new(mtm, rect(20.0, 56.0, PAGE_W, 530.0));
            root.addSubview(&p);
            p
        };
        let (p_general, p_drive, p_model, p_hist) = (mk_page(), mk_page(), mk_page(), mk_page());
        for p in [&p_drive, &p_model, &p_hist] {
            p.setHidden(true);
        }

        // ---- 一般 ----
        label(mtm, &p_general, "保存方法: Google Drive API", rect(4.0, 0.0, 400.0, 18.0), 12.0, false, theme::TEXT_SECONDARY, false);
        let c = card(mtm, &p_general, 0.0, 26.0, PAGE_W, 190.0);
        title(mtm, &c, "録音ホットキー");
        label(mtm, &c, "現在", rect(16.0, 44.0, 40.0, 18.0), 12.0, false, theme::TEXT_SECONDARY, false);
        let hotkey_display = label(mtm, &c, "", rect(60.0, 44.0, 300.0, 18.0), 13.0, true, theme::TEXT_PRIMARY, false);
        let hotkey = text_field(mtm, &c, rect(16.0, 70.0, 360.0, 24.0), false, "例: control+alt+KeyR");
        // SAFETY: delegate is weak; `handler` is owned by SettingsWindow for the window's lifetime.
        unsafe { hotkey.setDelegate(Some(ProtocolObject::from_ref(&*handler))) };
        handler.button(&c, "変更", rect(386.0, 69.0, 70.0, 26.0), Style::Primary, Act::Hotkey);
        let hotkey_error = label(mtm, &c, "", rect(16.0, 98.0, 568.0, 18.0), 12.0, false, theme::ERROR, false);
        let rec = handler.button(&c, "録音テスト", rect(16.0, 128.0, 110.0, 28.0), Style::Danger, Act::Do(UiAction::StartTestRecording));
        rec.setAccessibilityLabel(Some(&ns("録音テストを開始")));
        let mic = label(mtm, &c, "", rect(138.0, 133.0, 440.0, 18.0), 12.0, false, theme::TEXT_SECONDARY, false);
        label(mtm, &c, "ホットキーを押している間だけ録音します。", rect(16.0, 162.0, 560.0, 18.0), 12.0, false, theme::TEXT_SECONDARY, false);

        let c = card(mtm, &p_general, 0.0, 228.0, PAGE_W, 130.0);
        title(mtm, &c, "動作");
        let launch = unsafe {
            NSButton::checkboxWithTitle_target_action(&ns("ログイン時に起動"), Some(&*handler), Some(sel!(clicked:)), mtm)
        };
        launch.setFrame(rect(16.0, 42.0, 300.0, 22.0));
        launch.setTag(handler.register(Act::Launch));
        launch.setAttributedTitle(&colored_title("ログイン時に起動", theme::TEXT_PRIMARY));
        launch.setContentTintColor(Some(&ns_color(theme::ACCENT)));
        c.addSubview(&launch);
        label(mtm, &c, "音声ファイルの保持時間", rect(16.0, 78.0, 180.0, 18.0), 13.0, false, theme::TEXT_PRIMARY, false);
        let retention_label = label(mtm, &c, "", rect(200.0, 78.0, 90.0, 18.0), 13.0, true, theme::TEXT_PRIMARY, false);
        let stepper = NSStepper::initWithFrame(mtm.alloc(), rect(296.0, 74.0, 19.0, 28.0));
        stepper.setMinValue(0.0);
        stepper.setMaxValue(720.0);
        stepper.setIncrement(1.0);
        stepper.setValueWraps(false);
        unsafe {
            stepper.setTarget(Some(&handler));
            stepper.setAction(Some(sel!(clicked:)));
        }
        stepper.setTag(handler.register(Act::Retention));
        stepper.setAccessibilityLabel(Some(&ns("音声ファイルの保持時間（時間）")));
        c.addSubview(&stepper);
        label(mtm, &c, "保存完了後、この時間が過ぎた音声を削除します（0 = 保存後すぐ削除）。", rect(16.0, 104.0, 560.0, 18.0), 12.0, false, theme::TEXT_SECONDARY, false);

        let c = card(mtm, &p_general, 0.0, 370.0, PAGE_W, 110.0);
        title(mtm, &c, "ローカルデータ");
        handler.button(
            &c,
            "ローカルのメモ・音声をすべて削除",
            rect(16.0, 42.0, 250.0, 28.0),
            Style::Danger,
            Act::Confirm {
                message: "ローカルのメモ・音声をすべて削除しますか？".into(),
                info: "この端末内の音声・Markdown・履歴が削除されます。Google Drive に保存済みのファイルは削除されません。この操作は取り消せません。".into(),
                ok: "削除".into(),
                action: UiAction::DeleteAllLocalData,
            },
        );
        label(mtm, &c, "Google Drive 上のファイルは削除されません。", rect(16.0, 78.0, 560.0, 18.0), 12.0, false, theme::TEXT_SECONDARY, false);

        // ---- Google Drive ----
        let c = card(mtm, &p_drive, 0.0, 0.0, PAGE_W, 190.0);
        title(mtm, &c, "Google OAuth クライアント");
        label(mtm, &c, "クライアントID", rect(16.0, 42.0, 120.0, 18.0), 12.0, false, theme::TEXT_PRIMARY, false);
        let client_id = text_field(mtm, &c, rect(16.0, 62.0, 568.0, 24.0), false, "xxxx.apps.googleusercontent.com");
        // SAFETY: delegate is weak; `handler` is owned by SettingsWindow for the window's lifetime.
        unsafe { client_id.setDelegate(Some(ProtocolObject::from_ref(&*handler))) };
        label(mtm, &c, "クライアントシークレット", rect(16.0, 94.0, 200.0, 18.0), 12.0, false, theme::TEXT_PRIMARY, false);
        let secret = text_field(mtm, &c, rect(16.0, 114.0, 568.0, 24.0), true, "");
        // SAFETY: delegate is weak; `handler` is owned by SettingsWindow for the window's lifetime.
        unsafe { secret.setDelegate(Some(ProtocolObject::from_ref(&*handler))) };
        handler.button(&c, "保存", rect(16.0, 148.0, 80.0, 28.0), Style::Primary, Act::SaveClient);
        label(mtm, &c, "シークレットはキーチェーンに保存されます。", rect(108.0, 153.0, 470.0, 18.0), 12.0, false, theme::TEXT_SECONDARY, false);

        let c = card(mtm, &p_drive, 0.0, 202.0, PAGE_W, 110.0);
        title(mtm, &c, "接続");
        let status = label(mtm, &c, "", rect(16.0, 42.0, 568.0, 18.0), 13.0, false, theme::TEXT_PRIMARY, false);
        let connect = handler.button(&c, "Googleに接続", rect(16.0, 68.0, 130.0, 28.0), Style::Primary, Act::Do(UiAction::ConnectGoogle));
        let disconnect = handler.button(
            &c,
            "連携を解除",
            rect(158.0, 68.0, 100.0, 28.0),
            Style::Neutral,
            Act::Confirm {
                message: "Googleとの連携を解除しますか？".into(),
                info: "この端末の認証情報が削除されます。Google Drive に保存済みのファイルは残ります。".into(),
                ok: "連携を解除".into(),
                action: UiAction::DisconnectGoogle,
            },
        );

        let c = card(mtm, &p_drive, 0.0, 324.0, PAGE_W, 150.0);
        title(mtm, &c, "保存先フォルダ");
        let folder = label(mtm, &c, "", rect(16.0, 42.0, 568.0, 18.0), 13.0, false, theme::TEXT_PRIMARY, false);
        let pick = handler.button(&c, "保存先フォルダを選択", rect(16.0, 68.0, 170.0, 28.0), Style::Primary, Act::Do(UiAction::PickDriveFolder));
        let test_save = handler.button(&c, "保存テスト", rect(198.0, 68.0, 100.0, 28.0), Style::Neutral, Act::Do(UiAction::TestSave));
        let message = label(mtm, &c, "", rect(16.0, 106.0, 568.0, 36.0), 12.0, false, theme::TEXT_SECONDARY, true);

        // ---- 音声認識 ----
        let c = card(mtm, &p_model, 0.0, 0.0, PAGE_W, 130.0);
        title(mtm, &c, "モデル");
        let popup = NSPopUpButton::initWithFrame_pullsDown(mtm.alloc(), rect(16.0, 42.0, 140.0, 26.0), false);
        popup.addItemWithTitle(&ns("base"));
        popup.addItemWithTitle(&ns("small"));
        unsafe {
            popup.setTarget(Some(&handler));
            popup.setAction(Some(sel!(clicked:)));
        }
        popup.setTag(handler.register(Act::Model));
        popup.setAccessibilityLabel(Some(&ns("音声認識モデル")));
        c.addSubview(&popup);
        let model_status = label(mtm, &c, "", rect(170.0, 46.0, 410.0, 18.0), 13.0, false, theme::TEXT_PRIMARY, false);
        let download = handler.button(&c, "モデルをダウンロード", rect(16.0, 82.0, 170.0, 28.0), Style::Primary, Act::Do(UiAction::DownloadModel));

        let c = card(mtm, &p_model, 0.0, 142.0, PAGE_W, 130.0);
        title(mtm, &c, "whisper-cli");
        let cli = label(mtm, &c, "", rect(16.0, 42.0, 568.0, 18.0), 12.0, false, theme::TEXT_PRIMARY, false);
        cli.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        handler.button(&c, "選択…", rect(16.0, 76.0, 80.0, 28.0), Style::Neutral, Act::PickCli);
        handler.button(&c, "自動検出", rect(108.0, 76.0, 90.0, 28.0), Style::Neutral, Act::Do(UiAction::SetWhisperCliPath(None)));

        // ---- 履歴 ----
        let pending = label(mtm, &p_hist, "", rect(4.0, 6.0, 360.0, 20.0), 13.0, true, theme::TEXT_PRIMARY, false);
        let retry_all = handler.button(&p_hist, "すべて再試行", rect(PAGE_W - 120.0, 2.0, 120.0, 28.0), Style::Primary, Act::Do(UiAction::RetryAll));
        let c = card(mtm, &p_hist, 0.0, 40.0, PAGE_W, 470.0);
        let scroll = NSScrollView::initWithFrame(mtm.alloc(), rect(1.0, 1.0, PAGE_W - 2.0, 468.0));
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        let history_doc = FlippedView::new(mtm, rect(0.0, 0.0, PAGE_W - 2.0, 468.0));
        scroll.setDocumentView(Some(&history_doc));
        c.addSubview(&scroll);

        *handler.ivars().pages.borrow_mut() = vec![
            Retained::into_super(p_general),
            Retained::into_super(p_drive),
            Retained::into_super(p_model),
            Retained::into_super(p_hist),
        ];
        *handler.ivars().fields.borrow_mut() = Some(Fields { hotkey, client_id, secret, stepper, popup });

        SettingsWindow {
            mtm,
            window,
            handler,
            w: Widgets {
                hotkey_display,
                hotkey_error,
                mic,
                launch,
                retention_label,
                status,
                connect,
                disconnect,
                folder,
                pick,
                test_save,
                message,
                model_status,
                download,
                cli,
                pending,
                retry_all,
                history_doc,
            },
            history_fp: None,
            history_tags: Vec::new(),
        }
    }

    /// Show and bring to front (the app may temporarily activate itself for this window only).
    pub fn show(&mut self) {
        // Accessory apps are never frontmost on their own; activate only for this window.
        let mtm = MainThreadMarker::from(&*self.window);
        objc2_app_kit::NSApplication::sharedApplication(mtm).activate();
        self.window.center();
        self.window.makeKeyAndOrderFront(None);
    }

    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    /// Re-render with new data. Must be cheap enough to call on every job change.
    pub fn refresh(&mut self, vm: &SettingsViewModel) {
        let w = &self.w;
        set_text(&w.hotkey_display, &vm.hotkey_display);
        set_text(&w.hotkey_error, vm.hotkey_error.as_deref().unwrap_or(""));
        set_text(&w.mic, &format!("マイク: {}", vm.mic_status));
        w.launch.setState(if vm.launch_at_login { 1 } else { 0 });
        set_text(&w.retention_label, &format!("{} 時間", vm.audio_retention_hours));

        // Google Drive.
        let status = if vm.google_connected {
            match vm.google_account.as_deref() {
                Some(a) if !a.is_empty() => format!("接続済み: {a}"),
                _ => "接続済み".to_string(),
            }
        } else if vm.google_client_configured {
            "未接続".to_string()
        } else {
            "未接続（先にクライアントIDとシークレットを保存してください）".to_string()
        };
        set_text(&w.status, &status);
        let ct = if vm.google_connected { "再認証" } else { "Googleに接続" };
        style_button(&w.connect, ct, Style::Primary);
        w.connect.setAccessibilityLabel(Some(&ns(ct)));
        set_enabled(&w.connect, vm.google_client_configured);
        set_enabled(&w.disconnect, vm.google_connected);
        set_text(
            &w.folder,
            &match vm.drive_folder_name.as_deref() {
                Some(n) if !n.is_empty() => format!("保存先: {n}"),
                _ => "保存先: 未選択".to_string(),
            },
        );
        set_enabled(&w.pick, vm.google_connected);
        set_enabled(&w.test_save, vm.google_connected && vm.drive_folder_id.is_some());
        match &vm.drive_message {
            Some((m, is_err)) => {
                set_text(&w.message, m);
                let c = if *is_err { theme::ERROR } else { theme::SUCCESS };
                w.message.setTextColor(Some(&ns_color(c)));
            }
            None => set_text(&w.message, ""),
        }

        // Editable fields: never overwrite while the user is typing.
        if let Some(f) = self.handler.ivars().fields.borrow().as_ref() {
            if f.hotkey.currentEditor().is_none() {
                set_text(&f.hotkey, &vm.hotkey_spec);
            }
            if f.client_id.currentEditor().is_none() {
                set_text(&f.client_id, &vm.google_client_id);
            }
            f.secret.setPlaceholderString(Some(&ns(if vm.google_client_configured {
                "保存済み（変更する場合のみ入力）"
            } else {
                "クライアントシークレット"
            })));
            f.stepper.setDoubleValue(vm.audio_retention_hours as f64);
            if let Some(i) = ["base", "small"].iter().position(|m| *m == vm.model) {
                f.popup.selectItemAtIndex(i as isize);
            }
        }

        // 音声認識.
        set_text(&w.model_status, &format!("状態: {}", vm.model_status));
        set_enabled(&w.download, !vm.model_status.contains("導入済み") && !vm.model_status.contains("ダウンロード中"));
        set_text(
            &w.cli,
            &match &vm.whisper_cli {
                Some(p) => format!("パス: {p}"),
                None => "パス: 自動検出".to_string(),
            },
        );

        // 履歴.
        set_text(&w.pending, &format!("未保存: {} 件", vm.pending_count));
        let pc = if vm.pending_count > 0 { theme::WARNING } else { theme::TEXT_PRIMARY };
        w.pending.setTextColor(Some(&ns_color(pc)));
        set_enabled(&w.retry_all, vm.pending_count > 0);

        let fp: Fingerprint = vm
            .history
            .iter()
            .map(|j| {
                (
                    j.job_id.clone(),
                    j.status,
                    j.updated_at.timestamp_millis(),
                    j.remote_web_link.is_some(),
                    j.transcript_preview.clone(),
                )
            })
            .collect();
        if self.history_fp.as_ref() != Some(&fp) {
            self.history_fp = Some(fp);
            self.rebuild_history(&vm.history);
        }
    }

    fn rebuild_history(&mut self, jobs: &[Job]) {
        let mtm = self.mtm;
        let doc = self.w.history_doc.clone();
        for t in self.history_tags.drain(..) {
            self.handler.ivars().acts.borrow_mut().remove(&t);
        }
        let subs = doc.subviews();
        for v in subs.iter() {
            v.removeFromSuperview();
        }
        const ROW_H: f64 = 60.0;
        let width = PAGE_W - 2.0;
        let total = (jobs.len() as f64 * ROW_H).max(468.0);
        doc.setFrameSize(NSSize::new(width, total));
        if jobs.is_empty() {
            label(mtm, &doc, "履歴はまだありません", rect(16.0, 16.0, 400.0, 18.0), 13.0, false, theme::TEXT_SECONDARY, false);
        }
        for (i, j) in jobs.iter().enumerate() {
            let y = i as f64 * ROW_H;
            let (text, symbol, color) = status_info(j.status);
            label(mtm, &doc, &format_local(&j.created_at), rect(10.0, y + 10.0, 76.0, 18.0), 12.0, false, theme::TEXT_SECONDARY, false);
            let prev = j.transcript_preview.as_deref().map(|s| truncate_chars(s, 40)).unwrap_or_default();
            let prev = if prev.is_empty() { "（本文なし）".to_string() } else { prev };
            label(mtm, &doc, &prev, rect(90.0, y + 8.0, 206.0, 18.0), 13.0, false, theme::TEXT_PRIMARY, false);
            let dest = j.destination_name.as_deref().unwrap_or("Google Drive");
            label(mtm, &doc, dest, rect(90.0, y + 31.0, 206.0, 16.0), 11.0, false, theme::TEXT_SECONDARY, false);

            let iv = NSImageView::initWithFrame(mtm.alloc(), rect(304.0, y + 10.0, 16.0, 16.0));
            if let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(symbol), Some(&ns(text))) {
                iv.setImage(Some(&img));
            }
            iv.setContentTintColor(Some(&ns_color(color)));
            iv.setAccessibilityLabel(Some(&ns(text)));
            doc.addSubview(&iv);
            label(mtm, &doc, text, rect(324.0, y + 8.0, 112.0, 36.0), 11.0, false, color, true);

            let by = y + 18.0;
            let mut x = 442.0;
            if j.remote_web_link.is_some() {
                let b = self.handler.button(&doc, "開く", rect(x, by, 40.0, 24.0), Style::Neutral, Act::Do(UiAction::OpenJob(j.job_id.clone())));
                self.history_tags.push(b.tag());
            }
            x += 44.0;
            if can_retry(j.status) {
                let b = self.handler.button(&doc, "再試行", rect(x, by, 52.0, 24.0), Style::Primary, Act::Do(UiAction::RetryJob(j.job_id.clone())));
                self.history_tags.push(b.tag());
            }
            x += 56.0;
            let b = self.handler.button(
                &doc,
                "削除",
                rect(x, by, 40.0, 24.0),
                Style::Neutral,
                Act::Confirm {
                    message: "このメモを履歴から削除しますか？".into(),
                    info: "ローカルのデータが削除されます。Google Drive に保存済みのファイルは削除されません。".into(),
                    ok: "削除".into(),
                    action: UiAction::DeleteJob(j.job_id.clone()),
                },
            );
            self.history_tags.push(b.tag());
            let sep = FlippedView::new(mtm, rect(0.0, y + ROW_H - 1.0, width, 1.0));
            set_bg(&sep, theme::BORDER, None, 0.0);
            doc.addSubview(&sep);
        }
    }
}

fn set_text(f: &NSTextField, s: &str) {
    if f.stringValue().to_string() != s {
        f.setStringValue(&ns(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn formats_time() {
        let tz = FixedOffset::east_opt(9 * 3600).unwrap();
        let dt = tz.with_ymd_and_hms(2026, 10, 3, 7, 5, 9).unwrap();
        assert_eq!(format_md_hm(&dt), "10/03 07:05");
    }

    #[test]
    fn truncates() {
        assert_eq!(truncate_chars("あいうえお", 5), "あいうえお");
        assert_eq!(truncate_chars("あいうえおか", 5), "あいうえ…");
        assert_eq!(truncate_chars("a\nb", 10), "a b");
        assert_eq!(truncate_chars("  ", 3), "");
    }

    #[test]
    fn status_wording() {
        assert_eq!(status_info(JobStatus::RemoteConfirmed).0, "Google Driveに保存しました");
        assert_eq!(status_info(JobStatus::RetryWait).0, "保存待ち");
        assert_eq!(status_info(JobStatus::ActionRequired).2, theme::ERROR);
        assert!(can_retry(JobStatus::Error));
        assert!(!can_retry(JobStatus::RemoteConfirmed));
    }
}
