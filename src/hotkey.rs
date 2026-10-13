//! Global hotkey registration (global-hotkey crate) + release failsafe.
//! OWNER: recorder/hotkey agent.

use std::str::FromStr;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::GlobalHotKeyManager;

/// Wraps `GlobalHotKeyManager`. Must be created on the main thread.
pub struct HotkeyController {
    manager: GlobalHotKeyManager,
    current: Option<HotKey>,
}

fn map_error(e: &global_hotkey::Error) -> String {
    use global_hotkey::Error as E;
    match e {
        E::HotKeyParseError(_) | E::UnrecognizedHotKeyCode(_) | E::EmptyHotKeyToken(_) | E::UnexpectedHotKeyFormat(_) => {
            "ショートカットの形式が正しくありません".to_string()
        }
        E::AlreadyRegistered(_) | E::FailedToRegister(_) => "このショートカットは他のアプリで使用中です".to_string(),
        other => format!("ショートカットを登録できませんでした: {other}"),
    }
}

impl HotkeyController {
    pub fn new() -> anyhow::Result<Self> {
        let manager = GlobalHotKeyManager::new().map_err(|e| anyhow::anyhow!("GlobalHotKeyManager: {e}"))?;
        Ok(Self { manager, current: None })
    }

    /// Parse + register `spec` (e.g. "control+alt+KeyR"), unregistering the previous one.
    /// Err with a Japanese message on parse error or conflict (REC-05).
    /// On failure the previous hotkey stays registered.
    pub fn register(&mut self, spec: &str) -> Result<HotKey, String> {
        let hk = HotKey::from_str(spec).map_err(|_| "ショートカットの形式が正しくありません".to_string())?;
        if hk.mods.is_empty() {
            return Err("修飾キー(⌃⌥⇧⌘のいずれか)を含めてください".to_string());
        }
        if self.current == Some(hk) {
            return Ok(hk);
        }
        // Register the new one first so a conflict leaves the old binding intact.
        self.manager.register(hk).map_err(|e| map_error(&e))?;
        if let Some(old) = self.current.take() {
            if let Err(e) = self.manager.unregister(old) {
                log::warn!("failed to unregister old hotkey: {e}");
            }
        }
        self.current = Some(hk);
        Ok(hk)
    }

    pub fn current(&self) -> Option<HotKey> {
        self.current
    }

    /// The id to compare against `GlobalHotKeyEvent::id`.
    pub fn current_id(&self) -> Option<u32> {
        self.current.map(|h| h.id())
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
    fn CGEventSourceFlagsState(state_id: i32) -> u64;
}

const COMBINED_SESSION_STATE: i32 = 0;
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_ALT: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;

/// macOS virtual keycode (ANSI layout) for a physical key code, if known.
pub fn mac_keycode(code: Code) -> Option<u16> {
    use Code::*;
    Some(match code {
        KeyA => 0,
        KeyS => 1,
        KeyD => 2,
        KeyF => 3,
        KeyH => 4,
        KeyG => 5,
        KeyZ => 6,
        KeyX => 7,
        KeyC => 8,
        KeyV => 9,
        KeyB => 11,
        KeyQ => 12,
        KeyW => 13,
        KeyE => 14,
        KeyR => 15,
        KeyY => 16,
        KeyT => 17,
        Digit1 => 18,
        Digit2 => 19,
        Digit3 => 20,
        Digit4 => 21,
        Digit6 => 22,
        Digit5 => 23,
        Equal => 24,
        Digit9 => 25,
        Digit7 => 26,
        Minus => 27,
        Digit8 => 28,
        Digit0 => 29,
        BracketRight => 30,
        KeyO => 31,
        KeyU => 32,
        BracketLeft => 33,
        KeyI => 34,
        KeyP => 35,
        Enter => 36,
        KeyL => 37,
        KeyJ => 38,
        Quote => 39,
        KeyK => 40,
        Semicolon => 41,
        Backslash => 42,
        Comma => 43,
        Slash => 44,
        KeyN => 45,
        KeyM => 46,
        Period => 47,
        Tab => 48,
        Space => 49,
        Backquote => 50,
        Backspace => 51,
        Escape => 53,
        F1 => 122,
        F2 => 120,
        F3 => 99,
        F4 => 118,
        F5 => 96,
        F6 => 97,
        F7 => 98,
        F8 => 100,
        F9 => 101,
        F10 => 109,
        F11 => 103,
        F12 => 111,
        ArrowLeft => 123,
        ArrowRight => 124,
        ArrowDown => 125,
        ArrowUp => 126,
        _ => return None,
    })
}

fn flags_for(mods: Modifiers) -> u64 {
    let mut f = 0;
    if mods.contains(Modifiers::SHIFT) {
        f |= FLAG_SHIFT;
    }
    if mods.contains(Modifiers::CONTROL) {
        f |= FLAG_CONTROL;
    }
    if mods.contains(Modifiers::ALT) {
        f |= FLAG_ALT;
    }
    if mods.contains(Modifiers::SUPER) {
        f |= FLAG_COMMAND;
    }
    f
}

/// Failsafe (REC-04/REC-08): true while the hotkey's main key AND all its modifiers are physically held,
/// queried via CGEventSourceKeyState / CGEventSourceFlagsState (no Input Monitoring permission needed).
/// The app polls this ~every 100ms while recording and stops when it returns false.
pub fn is_physically_held(hotkey: &HotKey) -> bool {
    let need = flags_for(hotkey.mods);
    let have = unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) };
    if have & need != need {
        return false;
    }
    match mac_keycode(hotkey.key) {
        Some(kc) => unsafe { CGEventSourceKeyState(COMBINED_SESSION_STATE, kc) },
        None => true, // unknown key: only modifiers are checked to avoid false stops
    }
}

fn key_label(code: Code) -> String {
    let s = code.to_string();
    if let Some(rest) = s.strip_prefix("Key") {
        if rest.len() == 1 {
            return rest.to_string();
        }
    }
    if let Some(rest) = s.strip_prefix("Digit") {
        return rest.to_string();
    }
    match code {
        Code::Minus => "-".into(),
        Code::Equal => "=".into(),
        Code::BracketLeft => "[".into(),
        Code::BracketRight => "]".into(),
        Code::Backslash => "\\".into(),
        Code::Semicolon => ";".into(),
        Code::Quote => "'".into(),
        Code::Backquote => "`".into(),
        Code::Comma => ",".into(),
        Code::Period => ".".into(),
        Code::Slash => "/".into(),
        Code::ArrowLeft => "←".into(),
        Code::ArrowRight => "→".into(),
        Code::ArrowUp => "↑".into(),
        Code::ArrowDown => "↓".into(),
        _ => s,
    }
}

/// Human readable form for menus/settings, e.g. "⌃⌥R".
pub fn display_string(spec: &str) -> String {
    let Ok(hk) = HotKey::from_str(spec) else {
        return spec.to_string();
    };
    let mut s = String::new();
    if hk.mods.contains(Modifiers::CONTROL) {
        s.push('⌃');
    }
    if hk.mods.contains(Modifiers::ALT) {
        s.push('⌥');
    }
    if hk.mods.contains(Modifiers::SHIFT) {
        s.push('⇧');
    }
    if hk.mods.contains(Modifiers::SUPER) {
        s.push('⌘');
    }
    s.push_str(&key_label(hk.key));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_basic() {
        assert_eq!(display_string("control+alt+KeyR"), "⌃⌥R");
        assert_eq!(display_string("shift+super+Digit1"), "⇧⌘1");
        assert_eq!(display_string("Ctrl+Option+Space"), "⌃⌥Space");
        assert_eq!(display_string("super+shift+F5"), "⇧⌘F5");
        assert_eq!(display_string("garbage++"), "garbage++");
    }

    #[test]
    fn keycodes() {
        assert_eq!(mac_keycode(Code::KeyR), Some(15));
        assert_eq!(mac_keycode(Code::KeyA), Some(0));
        assert_eq!(mac_keycode(Code::Digit0), Some(29));
        assert_eq!(mac_keycode(Code::Space), Some(49));
        assert_eq!(mac_keycode(Code::F12), Some(111));
        assert_eq!(mac_keycode(Code::F1), Some(122));
        assert_eq!(mac_keycode(Code::Numpad0), None);
    }

    #[test]
    fn modifier_flags() {
        assert_eq!(flags_for(Modifiers::CONTROL | Modifiers::ALT), FLAG_CONTROL | FLAG_ALT);
        assert_eq!(flags_for(Modifiers::SUPER | Modifiers::SHIFT), FLAG_COMMAND | FLAG_SHIFT);
    }

    #[test]
    fn physical_state_does_not_crash() {
        let hk = HotKey::from_str("control+alt+KeyR").unwrap();
        // Nobody is holding it in CI.
        let _ = is_physically_held(&hk);
    }
}
