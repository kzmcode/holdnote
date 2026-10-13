//! Recording floating overlay: non-activating, click-through NSPanel pill at the top-center of the
//! active screen (requirements §4.4/§4.5). Main thread only. Colors only from `ui::theme`.
//! OWNER: overlay agent.

use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSBackingStoreType, NSColor, NSEvent, NSFont, NSFontWeightMedium,
    NSFontWeightSemibold, NSImage, NSImageView, NSLineBreakMode, NSPanel, NSScreen,
    NSStatusWindowLevel, NSTextAlignment, NSTextField, NSView, NSWindowCollectionBehavior,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CALayer, CATransaction};

use super::theme::{self, Rgb};

#[derive(Debug, Clone, PartialEq)]
pub enum OverlayState {
    Hidden,
    /// "録音中 00:12", coral dot/ring + white mic, level bars, "キーを離すと終了".
    Recording { elapsed: Duration, level: f32 },
    /// "文字起こし中…" (accent blue)
    Transcribing,
    /// "保存中…" (accent blue)
    Saving,
    /// "Google Driveに保存しました" (green ✓)
    SavedRemote,
    /// "保存待ち" + reason (amber)
    RetryWait(String),
    /// "保存できませんでした" + reason (red + ⚠)
    ActionRequired(String),
    /// Short neutral info, e.g. "録音が短すぎるため破棄しました" (secondary text)
    Info(String),
}

const W: f64 = theme::OVERLAY_WIDTH;
const H: f64 = theme::OVERLAY_HEIGHT;
const RING: f64 = 40.0;
const RING_X: f64 = 14.0;
const TEXT_X: f64 = 66.0;
const TEXT_W: f64 = W - TEXT_X - 14.0;
const BAR_W: f64 = 5.0;
const BAR_PITCH: f64 = 11.0;
const BAR_MIN: f64 = 3.0;
const BAR_MAX: f64 = 16.0;
const BAR_Y: f64 = 12.0;

/// Per-state presentation.
struct Look {
    symbol: &'static str,
    tint: Rgb,
    ring: Rgb,
    title: &'static str,
    detail: Option<String>,
}

fn look(state: &OverlayState) -> Option<Look> {
    use OverlayState::*;
    let l = |symbol, color, title, detail: Option<String>| Look {
        symbol,
        tint: color,
        ring: color,
        title,
        detail: detail.filter(|d| !d.is_empty()),
    };
    Some(match state {
        Hidden => return None,
        Recording { .. } => Look {
            symbol: "mic.fill",
            tint: theme::TEXT_PRIMARY,
            ring: theme::RECORDING,
            title: "録音中",
            detail: None,
        },
        Transcribing => l("waveform", theme::ACCENT, "文字起こし中…", None),
        Saving => l("arrow.up.circle", theme::ACCENT, "保存中…", None),
        SavedRemote => l("checkmark.circle.fill", theme::SUCCESS, "Google Driveに保存しました", None),
        RetryWait(r) => l("clock.badge.exclamationmark", theme::WARNING, "保存待ち", Some(r.clone())),
        ActionRequired(r) => {
            l("exclamationmark.triangle.fill", theme::ERROR, "保存できませんでした", Some(r.clone()))
        }
        Info(t) => l("info.circle", theme::TEXT_SECONDARY, "", Some(t.clone())),
    })
}

fn hide_after(state: &OverlayState) -> Option<Duration> {
    match state {
        OverlayState::Hidden | OverlayState::Recording { .. } => None,
        OverlayState::ActionRequired(_) => Some(Duration::from_secs(4)),
        _ => Some(Duration::from_secs(2)),
    }
}

fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

fn cg(c: Rgb, alpha: f64) -> Retained<objc2_core_graphics::CGColor> {
    theme::ns_color_alpha(c, alpha).CGColor()
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

pub struct Overlay {
    mtm: MainThreadMarker,
    panel: Retained<NSPanel>,
    root: Retained<NSView>,
    pill: Retained<CALayer>,
    ring: Retained<CALayer>,
    icon: Retained<NSImageView>,
    title: Retained<NSTextField>,
    time: Retained<NSTextField>,
    hint: Retained<NSTextField>,
    detail: Retained<NSTextField>,
    bars: Vec<Retained<CALayer>>,
    history: Vec<f32>,
    state: OverlayState,
    hide_at: Option<Instant>,
    visible: bool,
    a11y_label: String,
}

fn label(mtm: MainThreadMarker, size: f64, weight: f64, color: Rgb, mono: bool) -> Retained<NSTextField> {
    let t = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    let font = if mono {
        NSFont::monospacedDigitSystemFontOfSize_weight(size, weight)
    } else {
        NSFont::systemFontOfSize_weight(size, weight)
    };
    t.setFont(Some(&font));
    t.setTextColor(Some(&theme::ns_color(color)));
    t.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    t
}

impl Overlay {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let style = NSWindowStyleMask::NonactivatingPanel | NSWindowStyleMask::Borderless;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            rect(0.0, 0.0, W, H),
            style,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setLevel(NSStatusWindowLevel);
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(false);
        panel.setCanHide(false);
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setOpaque(false);
        panel.setHasShadow(true);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        let root = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, W, H));
        root.setWantsLayer(true);
        let pill = root.layer().expect("layer-backed view has a layer");
        pill.setCornerRadius(theme::OVERLAY_RADIUS);
        pill.setBorderWidth(1.0);
        pill.setBorderColor(Some(&cg(theme::BORDER, 1.0)));
        pill.setMasksToBounds(false);

        // Ring (+ glow) behind the icon.
        let ring = CALayer::new();
        ring.setFrame(rect(RING_X, (H - RING) / 2.0, RING, RING));
        ring.setCornerRadius(RING / 2.0);
        ring.setBorderWidth(2.0);
        ring.setShadowOffset(NSSize::new(0.0, 0.0));
        ring.setShadowRadius(7.0);
        pill.addSublayer(&ring);

        let mut bars = Vec::new();
        for i in 0..theme::LEVEL_BAR_COUNT {
            let b = CALayer::new();
            b.setFrame(rect(TEXT_X + i as f64 * BAR_PITCH, BAR_Y, BAR_W, BAR_MIN));
            b.setCornerRadius(BAR_W / 2.0);
            b.setBackgroundColor(Some(&cg(theme::RECORDING, 1.0)));
            pill.addSublayer(&b);
            bars.push(b);
        }

        let icon = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            rect(RING_X + 8.0, (H - RING) / 2.0 + 8.0, RING - 16.0, RING - 16.0),
        );
        root.addSubview(&icon);

        let title = label(mtm, 15.0, unsafe { NSFontWeightSemibold }, theme::TEXT_PRIMARY, false);
        let time = label(mtm, 15.0, unsafe { NSFontWeightMedium }, theme::TEXT_PRIMARY, true);
        let hint = label(mtm, 11.0, unsafe { NSFontWeightMedium }, theme::TEXT_SECONDARY, false);
        hint.setAlignment(NSTextAlignment::Right);
        hint.setStringValue(&NSString::from_str("キーを離すと終了"));
        let detail = label(mtm, 12.0, unsafe { NSFontWeightMedium }, theme::TEXT_SECONDARY, false);
        for v in [&title, &time, &hint, &detail] {
            root.addSubview(v);
        }
        panel.setContentView(Some(&root));

        Overlay {
            mtm,
            panel,
            root,
            pill,
            ring,
            icon,
            title,
            time,
            hint,
            detail,
            bars,
            history: vec![0.0; theme::LEVEL_BAR_COUNT],
            state: OverlayState::Hidden,
            hide_at: None,
            visible: false,
            a11y_label: String::new(),
        }
    }

    /// Show a state. Transient states (Transcribing/Saving/SavedRemote/RetryWait/ActionRequired/Info)
    /// auto-hide after ~2 s (ActionRequired ~4 s) — handled via `tick`. Must never block.
    pub fn set_state(&mut self, state: OverlayState) {
        if state == OverlayState::Hidden {
            self.hide();
            return;
        }
        let was_recording = matches!(self.state, OverlayState::Recording { .. });
        let is_recording = matches!(state, OverlayState::Recording { .. });
        self.hide_at = hide_after(&state).map(|d| Instant::now() + d);
        if is_recording && !was_recording {
            self.history.iter_mut().for_each(|v| *v = 0.0);
        }
        let reposition = !self.visible;
        self.state = state;
        self.apply_layout();
        if let OverlayState::Recording { elapsed, level } = self.state.clone() {
            self.push_level(level);
            self.apply_recording(elapsed);
        }
        if reposition {
            self.position();
        }
        if !self.visible {
            self.panel.orderFrontRegardless();
            self.visible = true;
        }
    }

    /// Cheap update while recording (≤10 fps). No-op if not in Recording state.
    pub fn update_recording(&mut self, elapsed: Duration, level: f32) {
        if !matches!(self.state, OverlayState::Recording { .. }) || !self.visible {
            return;
        }
        self.state = OverlayState::Recording { elapsed, level };
        self.push_level(level);
        self.apply_recording(elapsed);
    }

    /// Call from the event loop; handles auto-hide. Returns the next instant the overlay needs a tick, if any.
    pub fn tick(&mut self) -> Option<Instant> {
        if let Some(at) = self.hide_at {
            if Instant::now() >= at {
                self.hide();
                return None;
            }
        }
        self.hide_at
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    fn hide(&mut self) {
        self.state = OverlayState::Hidden;
        self.hide_at = None;
        if self.visible {
            self.panel.orderOut(None);
            self.visible = false;
        }
    }

    fn push_level(&mut self, level: f32) {
        self.history.remove(0);
        self.history.push(level.clamp(0.0, 1.0));
    }

    /// Top-center of the screen containing the mouse (fallback: main screen), below menu bar/notch.
    fn position(&self) {
        let mtm = self.mtm;
        let mouse = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        let mut target = None;
        for s in screens.iter() {
            let f = s.frame();
            if mouse.x >= f.origin.x
                && mouse.x < f.origin.x + f.size.width
                && mouse.y >= f.origin.y
                && mouse.y < f.origin.y + f.size.height
            {
                target = Some(s);
                break;
            }
        }
        let Some(screen) = target.or_else(|| NSScreen::mainScreen(mtm)) else {
            return;
        };
        let vf = screen.visibleFrame();
        let x = vf.origin.x + (vf.size.width - W) / 2.0;
        let y = vf.origin.y + vf.size.height - theme::OVERLAY_TOP_MARGIN - H;
        self.panel.setFrame_display(rect(x.round(), y.round(), W, H), false);
    }

    fn apply_layout(&mut self) {
        let Some(look) = look(&self.state) else { return };
        let recording = matches!(self.state, OverlayState::Recording { .. });
        let reduce_transparency =
            NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency();

        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.pill
            .setBackgroundColor(Some(&cg(theme::SURFACE, if reduce_transparency { 1.0 } else { 0.96 })));
        self.ring.setBackgroundColor(Some(&cg(look.ring, 0.16)));
        self.ring.setBorderColor(Some(&cg(look.ring, 1.0)));
        // Weak static outer glow (no pulsing, so nothing to suppress under Reduce Motion).
        self.ring.setShadowColor(Some(&cg(look.ring, 1.0)));
        self.ring.setShadowOpacity(if recording { 0.55 } else { 0.0 });
        for b in &self.bars {
            b.setHidden(!recording);
        }
        CATransaction::commit();

        let img = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(look.symbol),
            Some(&NSString::from_str(look.title)),
        );
        self.icon.setImage(img.as_deref());
        self.icon.setContentTintColor(Some(&theme::ns_color(look.tint)));

        self.hint.setHidden(!recording);
        self.time.setHidden(!recording);
        let mid = |h: f64| (H - h) / 2.0;
        let label_text;
        if recording {
            self.title.setStringValue(&NSString::from_str(look.title));
            self.title.setFrame(rect(TEXT_X, 38.0, 50.0, 20.0));
            self.time.setFrame(rect(TEXT_X + 52.0, 38.0, 48.0, 20.0));
            self.hint.setFrame(rect(TEXT_X + 102.0, 40.0, TEXT_W - 102.0, 15.0));
            self.detail.setHidden(true);
            label_text = look.title.to_string();
        } else if look.title.is_empty() {
            // Info: a single primary-colored line.
            let t = look.detail.clone().unwrap_or_default();
            self.title.setStringValue(&NSString::from_str(&t));
            self.title.setFrame(rect(TEXT_X, mid(20.0), TEXT_W, 20.0));
            self.detail.setHidden(true);
            label_text = t;
        } else {
            self.title.setStringValue(&NSString::from_str(look.title));
            match &look.detail {
                Some(d) => {
                    self.title.setFrame(rect(TEXT_X, 35.0, TEXT_W, 20.0));
                    self.detail.setStringValue(&NSString::from_str(d));
                    self.detail.setFrame(rect(TEXT_X, 15.0, TEXT_W, 17.0));
                    self.detail.setHidden(false);
                    label_text = format!("{}、{}", look.title, d);
                }
                None => {
                    self.title.setFrame(rect(TEXT_X, mid(20.0), TEXT_W, 20.0));
                    self.detail.setHidden(true);
                    label_text = look.title.to_string();
                }
            }
        }

        // Accessibility label: only on state-text change, never per level tick.
        if label_text != self.a11y_label {
            let ns = NSString::from_str(&label_text);
            self.root.setAccessibilityLabel(Some(&ns));
            self.a11y_label = label_text;
        }
    }

    fn apply_recording(&mut self, elapsed: Duration) {
        self.time.setStringValue(&NSString::from_str(&fmt_elapsed(elapsed)));
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for (b, lv) in self.bars.iter().zip(self.history.iter()) {
            let h = BAR_MIN + (BAR_MAX - BAR_MIN) * (*lv as f64).sqrt();
            b.setFrame(rect(b.frame().origin.x, BAR_Y, BAR_W, h));
        }
        CATransaction::commit();
    }
}

/// Dev helper for `holdnote --overlay-demo`: blocks the main thread, cycling through every overlay
/// state on screen (recording with a synthetic level, then each transient state), then returns.
pub fn demo(mtm: MainThreadMarker) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::{NSDate, NSRunLoop};

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let mut ov = Overlay::new(mtm);

    let pump = |secs: f64| {
        let until = NSDate::dateWithTimeIntervalSinceNow(secs);
        NSRunLoop::currentRunLoop().runUntilDate(&until);
    };

    ov.set_state(OverlayState::Recording { elapsed: Duration::ZERO, level: 0.0 });
    for i in 0..60u32 {
        let t = i as f32 * 0.1;
        let level = ((t * 5.0).sin() * 0.5 + 0.5) * ((t * 1.7).cos() * 0.4 + 0.6);
        ov.update_recording(Duration::from_millis(i as u64 * 100), level);
        pump(0.1);
    }
    let states = [
        OverlayState::Transcribing,
        OverlayState::Saving,
        OverlayState::SavedRemote,
        OverlayState::RetryWait("オフラインです".into()),
        OverlayState::ActionRequired("Googleの再認証が必要です".into()),
        OverlayState::Info("録音が短すぎるため破棄しました".into()),
    ];
    for s in states {
        ov.set_state(s);
        while ov.tick().is_some() {
            pump(0.1);
        }
        pump(0.4);
    }
}
