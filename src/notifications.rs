//! User notifications (macOS Notification Center). Must not block the caller.
//! OWNER: app agent.

use std::process::Command;

/// Escape a string for use inside an AppleScript double-quoted literal.
pub(crate) fn escape_applescript(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\r' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn build_script(title: &str, body: &str) -> String {
    format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(body),
        escape_applescript(title)
    )
}

/// Post a notification (title, body). Fire-and-forget.
pub fn notify(title: &str, body: &str) {
    let script = build_script(title, body);
    std::thread::spawn(move || {
        match Command::new("/usr/bin/osascript").arg("-e").arg(script).output() {
            Ok(o) if !o.status.success() => log::warn!("notification failed (osascript exit {:?})", o.status.code()),
            Err(e) => log::warn!("notification failed: {e}"),
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_and_backslashes() {
        assert_eq!(escape_applescript(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(escape_applescript("l1\nl2"), "l1 l2");
        assert_eq!(escape_applescript("日本語"), "日本語");
    }

    #[test]
    fn script_shape() {
        let s = build_script("HoldNote", "say \"hi\"");
        assert_eq!(s, r#"display notification "say \"hi\"" with title "HoldNote""#);
    }

    #[test]
    fn injection_is_neutralised() {
        let s = build_script("t", "\" & (do shell script \"x\") & \"");
        assert!(s.contains("\\\" & (do shell script \\\"x\\\") & \\\""));
    }
}
