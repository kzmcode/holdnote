//! Recording floating overlay: non-activating, click-through NSPanel pill at the top-center of the
//! active screen (requirements §4.4/§4.5). Main thread only. Colors only from `ui::theme`.
//! OWNER: overlay agent.

use std::time::Duration;

use objc2::MainThreadMarker;

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

pub struct Overlay {
    // implementation detail
}

impl Overlay {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let _ = mtm;
        todo!()
    }
    /// Show a state. Transient states (Transcribing/Saving/SavedRemote/RetryWait/ActionRequired/Info)
    /// auto-hide after ~2 s (ActionRequired ~4 s) — handled via `tick`. Must never block.
    pub fn set_state(&mut self, state: OverlayState) {
        let _ = state;
        todo!()
    }
    /// Cheap update while recording (≤10 fps). No-op if not in Recording state.
    pub fn update_recording(&mut self, elapsed: Duration, level: f32) {
        let _ = (elapsed, level);
        todo!()
    }
    /// Call from the event loop; handles auto-hide. Returns the next instant the overlay needs a tick, if any.
    pub fn tick(&mut self) -> Option<std::time::Instant> {
        todo!()
    }
    pub fn is_visible(&self) -> bool {
        todo!()
    }
}
