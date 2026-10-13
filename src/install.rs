//! Launch-at-login (LaunchAgent plist in ~/Library/LaunchAgents) and bundle helpers.
//! OWNER: app agent.

use std::path::{Path, PathBuf};

use crate::paths::BUNDLE_ID;

fn launch_agent_path() -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("ホームディレクトリが見つかりません"))?;
    Ok(home.join("Library/LaunchAgents").join(format!("{BUNDLE_ID}.plist")))
}

pub(crate) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// LaunchAgent plist text for `label` running `program_args` at login.
pub(crate) fn plist_contents(label: &str, program_args: &[String]) -> String {
    let args: String = program_args
        .iter()
        .map(|a| format!("        <string>{}</string>\n", xml_escape(a)))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>Label</key>\n\
    <string>{}</string>\n\
    <key>ProgramArguments</key>\n\
    <array>\n{}    </array>\n\
    <key>RunAtLoad</key>\n\
    <true/>\n\
    <key>ProcessType</key>\n\
    <string>Interactive</string>\n\
</dict>\n\
</plist>\n",
        xml_escape(label),
        args
    )
}

/// Given an executable path, return the enclosing `.app` if it is `X.app/Contents/MacOS/<exe>`.
pub(crate) fn bundle_of_exe(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    if macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension().map(|e| e == "app").unwrap_or(false)
    {
        Some(app.to_path_buf())
    } else {
        None
    }
}

pub fn set_launch_at_login(enabled: bool) -> anyhow::Result<()> {
    let path = launch_agent_path()?;
    if enabled {
        let exe = std::env::current_exe()?;
        let plist = plist_contents(BUNDLE_ID, &[exe.to_string_lossy().into_owned()]);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("plist.tmp");
        std::fs::write(&tmp, plist)?;
        std::fs::rename(&tmp, &path)?;
    } else if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn is_launch_at_login() -> bool {
    launch_agent_path().map(|p| p.exists()).unwrap_or(false)
}

/// Path to the running .app bundle (None when run via `cargo run`).
pub fn bundle_path() -> Option<PathBuf> {
    bundle_of_exe(&std::env::current_exe().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_has_required_keys_and_escapes() {
        let p = plist_contents("io.x.App", &["/Apps/A&B <x>.app/Contents/MacOS/holdnote".into()]);
        assert!(p.contains("<key>RunAtLoad</key>\n<true/>"));
        assert!(p.contains("<string>io.x.App</string>"));
        assert!(p.contains("/Apps/A&amp;B &lt;x&gt;.app/Contents/MacOS/holdnote"));
        assert!(p.starts_with("<?xml"));
    }

    #[test]
    fn bundle_detection() {
        let b = bundle_of_exe(Path::new("/Applications/HoldNote.app/Contents/MacOS/holdnote"));
        assert_eq!(b, Some(PathBuf::from("/Applications/HoldNote.app")));
        assert_eq!(bundle_of_exe(Path::new("/x/target/debug/holdnote")), None);
    }
}
