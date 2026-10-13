//! Global hotkey registration (global-hotkey crate) + release failsafe.
//! OWNER: recorder/hotkey agent.

use global_hotkey::hotkey::HotKey;

/// Wraps `GlobalHotKeyManager`. Must be created on the main thread.
pub struct HotkeyController {
    // implementation detail
}

impl HotkeyController {
    pub fn new() -> anyhow::Result<Self> {
        todo!()
    }
    /// Parse + register `spec` (e.g. "control+alt+KeyR"), unregistering the previous one.
    /// Err with a Japanese message on parse error or conflict (REC-05).
    pub fn register(&mut self, spec: &str) -> Result<HotKey, String> {
        let _ = spec;
        todo!()
    }
    pub fn current(&self) -> Option<HotKey> {
        todo!()
    }
    /// The id to compare against `GlobalHotKeyEvent::id`.
    pub fn current_id(&self) -> Option<u32> {
        todo!()
    }
}

/// Failsafe (REC-04/REC-08): true while the hotkey's main key AND all its modifiers are physically held,
/// queried via CGEventSourceKeyState / CGEventSourceFlagsState (no Input Monitoring permission needed).
/// The app polls this ~every 100ms while recording and stops when it returns false.
pub fn is_physically_held(hotkey: &HotKey) -> bool {
    let _ = hotkey;
    todo!()
}

/// Human readable form for menus/settings, e.g. "⌃⌥R".
pub fn display_string(spec: &str) -> String {
    let _ = spec;
    todo!()
}
