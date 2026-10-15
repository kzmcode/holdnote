//! `holdnote --doctor`: read-only diagnostic report (Japanese) printed to stdout.
//! Never starts the GUI, never writes files/settings, never prompts, never uses the network.
//! Never prints tokens, the client secret, or transcript text.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Warn,
    Bad,
}

struct Report {
    lines: Vec<String>,
    warns: usize,
    bads: usize,
}

impl Report {
    fn new() -> Self {
        Self { lines: Vec::new(), warns: 0, bads: 0 }
    }
    fn section(&mut self, title: &str) {
        if !self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.lines.push(format!("■ {title}"));
    }
    fn add(&mut self, level: Level, text: impl AsRef<str>, hint: Option<&str>) {
        let mark = match level {
            Level::Ok => "✔",
            Level::Warn => {
                self.warns += 1;
                "⚠"
            }
            Level::Bad => {
                self.bads += 1;
                "✘"
            }
        };
        let mut s = format!("  {mark} {}", text.as_ref());
        if level != Level::Ok {
            if let Some(h) = hint {
                s.push_str(&format!("\n      → {h}"));
            }
        }
        self.lines.push(s);
    }
    fn ok(&mut self, t: impl AsRef<str>) {
        self.add(Level::Ok, t, None);
    }
}

/// Mask a Google client ID: first 6 chars + "…" + ".apps.googleusercontent.com" if present.
pub(crate) fn mask_client_id(id: &str) -> String {
    const SUFFIX: &str = ".apps.googleusercontent.com";
    let head: String = id.chars().take(6).collect();
    if id.ends_with(SUFFIX) {
        format!("{head}…{SUFFIX}")
    } else {
        format!("{head}…")
    }
}

/// True when an input device name looks like a virtual/aggregate device.
pub(crate) fn is_virtual_device(name: &str) -> bool {
    let n = name.to_lowercase();
    ["blackhole", "loopback", "soundflower", "teams", "zoom", "aggregate", "virtual"]
        .iter()
        .any(|k| n.contains(k))
}

pub(crate) fn summary_line(bads: usize, warns: usize) -> String {
    if bads == 0 && warns == 0 {
        "問題は見つかりませんでした".to_string()
    } else {
        format!("{bads} 件の問題、{warns} 件の注意があります")
    }
}

pub(crate) fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Same location as `paths::app_support_dir()` but without creating it.
fn support_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join(crate::paths::APP_NAME)
}

fn macos_version() -> Option<String> {
    let out = std::process::Command::new("sw_vers").arg("-productVersion").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Run `<cli> --help` with a short timeout; Ok(()) if it started and exited.
fn probe_cli(cli: &Path) -> Result<(), String> {
    let mut child = std::process::Command::new(cli)
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if start.elapsed() > Duration::from_secs(3) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("3 秒以内に応答しませんでした".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn app_section(r: &mut Report) {
    r.section("アプリ");
    r.ok(format!("バージョン: {}", env!("CARGO_PKG_VERSION")));
    match std::env::current_exe() {
        Ok(exe) => {
            r.ok(format!("実行ファイル: {}", exe.display()));
            match crate::install::bundle_of_exe(&exe) {
                Some(b) => r.ok(format!(".app バンドル: {}", b.display())),
                None => r.add(
                    Level::Warn,
                    ".app バンドルから実行されていません（開発ビルド？）",
                    Some("/Applications/HoldNote.app から起動してください"),
                ),
            }
        }
        Err(e) => r.add(Level::Warn, format!("実行ファイルのパスを取得できません: {e}"), None),
    }
    match macos_version() {
        Some(v) => r.ok(format!("macOS: {v}")),
        None => r.add(Level::Warn, "macOS のバージョンを取得できません", None),
    }
    r.ok(format!("CPU: {}", std::env::consts::ARCH));
}

fn settings_section(r: &mut Report) -> Settings {
    r.section("設定ファイル");
    let path = support_dir().join("settings.toml");
    r.ok(format!("パス: {}", path.display()));
    let mut settings = Settings::default();
    match std::fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Settings>(&text) {
            Ok(s) => {
                settings = s;
                r.ok("読み込み: 正常");
            }
            Err(e) => r.add(
                Level::Bad,
                format!("解析エラー（既定値で表示します）: {}", e.message()),
                Some("設定ファイルを修正するか削除してください（起動時に .bak へ退避されます）"),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => r.add(
            Level::Warn,
            "設定ファイルがまだありません（既定値を使用）",
            Some("アプリを一度起動して初期設定を完了してください"),
        ),
        Err(e) => r.add(Level::Bad, format!("読み込めません: {e}"), Some("ファイルの権限を確認してください")),
    }
    r.ok(format!(
        "ホットキー: {}（{}）",
        crate::hotkey::display_string(&settings.hotkey),
        settings.hotkey
    ));
    settings
}

fn mic_section(r: &mut Report) {
    r.section("マイク");
    match crate::recorder::default_input_name() {
        Some(name) if is_virtual_device(&name) => r.add(
            Level::Warn,
            format!("既定の入力デバイス: {name}（仮想デバイスの可能性）"),
            Some("システム設定 > サウンド > 入力 で実マイクを選択してください"),
        ),
        Some(name) => r.ok(format!("既定の入力デバイス: {name}")),
        None => r.add(
            Level::Bad,
            "既定の入力デバイスが見つかりません",
            Some("システム設定 > サウンド > 入力 を確認してください"),
        ),
    }
    use crate::recorder::MicPermission::*;
    let note = "（Terminal から実行した場合は Terminal アプリの権限を示します）";
    match crate::recorder::mic_permission() {
        Granted => r.ok(format!("マイク権限: 許可済み {note}")),
        Denied => r.add(
            Level::Bad,
            format!("マイク権限: 拒否 {note}"),
            Some("システム設定 > プライバシーとセキュリティ > マイク で HoldNote を許可してください"),
        ),
        NotDetermined => r.add(
            Level::Warn,
            format!("マイク権限: 未確認 {note}"),
            Some("HoldNote を起動して初回の録音時に許可してください"),
        ),
    }
}

fn whisper_section(r: &mut Report, s: &Settings) {
    r.section("文字起こし");
    match crate::transcriber::find_whisper_cli(s.whisper_cli_path.as_deref()) {
        None => r.add(
            Level::Bad,
            "whisper-cli が見つかりません",
            Some("brew install whisper.cpp を実行するか、アプリを再インストールしてください"),
        ),
        Some(cli) => {
            let bundled = crate::install::bundle_path().is_some_and(|b| cli.starts_with(b));
            r.ok(format!("whisper-cli: {}（{}）", cli.display(), if bundled { "同梱" } else { "同梱ではない" }));
            match probe_cli(&cli) {
                Ok(()) => r.ok("whisper-cli --help: 起動できました"),
                Err(e) => r.add(
                    Level::Bad,
                    format!("whisper-cli を起動できません: {e}"),
                    Some("アプリを再インストールするか brew reinstall whisper.cpp を実行してください"),
                ),
            }
        }
    }
    match crate::transcriber::model_info(&s.model) {
        None => r.add(
            Level::Bad,
            format!("未対応のモデル名: {}", s.model),
            Some("設定 > 音声認識 でモデルを選び直してください"),
        ),
        Some(info) => {
            // Size-only check, done here (not via transcriber::model_status) so no directory is created.
            let path = support_dir().join("models").join(info.file_name);
            let hint = "設定 > 音声認識 > モデルをダウンロード";
            match std::fs::metadata(&path) {
                Ok(m) if m.is_file() && m.len() == info.size_bytes => {
                    r.ok(format!("モデル: {}（{}, {}）", s.model, path.display(), human_size(m.len())))
                }
                Ok(m) if m.is_file() => r.add(
                    Level::Bad,
                    format!(
                        "モデル: {} が破損しています（{} / 期待 {}）: {}",
                        s.model,
                        human_size(m.len()),
                        human_size(info.size_bytes),
                        path.display()
                    ),
                    Some(hint),
                ),
                _ => r.add(Level::Bad, format!("モデル: {} がありません: {}", s.model, path.display()), Some(hint)),
            }
        }
    }
    r.ok(format!("言語: {}", s.language));
}

fn drive_section(r: &mut Report, s: &Settings) {
    r.section("Google Drive");
    if s.google_client_id.is_empty() {
        r.add(Level::Bad, "クライアントID: 未設定", Some("設定 > Google Drive でクライアントIDを入力してください"));
    } else {
        r.ok(format!("クライアントID: {}", mask_client_id(&s.google_client_id)));
    }
    if s.google_client_secret.is_empty() {
        r.add(Level::Bad, "クライアントシークレット: 未設定", Some("設定 > Google Drive で入力してください"));
    } else {
        r.ok("クライアントシークレット: 設定済み");
    }
    match crate::security::load_tokens() {
        Ok(Some(_)) => r.ok("接続: 接続済み"),
        Ok(None) => r.add(
            Level::Warn,
            "接続: 未接続",
            Some("設定 > Google Drive から Google アカウントに接続してください"),
        ),
        Err(e) => r.add(
            Level::Warn,
            format!("接続: Keychain を確認できません（{e}）"),
            Some("Keychain へのアクセスを許可してください"),
        ),
    }
    match &s.drive_folder_id {
        Some(id) => r.ok(format!(
            "保存先フォルダ: {}（{id}）",
            s.drive_folder_name.as_deref().unwrap_or("名前なし")
        )),
        None => r.add(
            Level::Warn,
            "保存先フォルダ: 未設定",
            Some("設定 > Google Drive で保存先フォルダを選択してください"),
        ),
    }
    match &s.google_account_email {
        Some(e) if !e.is_empty() => r.ok(format!("アカウント: {e}")),
        _ => r.add(Level::Warn, "アカウント: 記録なし", None),
    }
}

fn jobs_section(r: &mut Report) {
    r.section("ジョブ");
    let path = support_dir().join("jobs.sqlite3");
    if !path.exists() {
        r.ok("ジョブDBはまだありません（録音履歴なし）");
        return;
    }
    let res: Result<(), rusqlite::Error> = (|| {
        let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM jobs GROUP BY status")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let count = |names: &[&str]| -> i64 {
            rows.iter().filter(|(s, _)| names.contains(&s.as_str())).map(|(_, n)| n).sum()
        };
        let pending: i64 = rows
            .iter()
            .filter(|(s, _)| crate::jobs::JobStatus::parse(s).is_some_and(|j| j.is_pending()))
            .map(|(_, n)| n)
            .sum();
        let failed = count(&["error"]);
        let action = count(&["action_required"]);
        let saved = count(&["remote_confirmed"]);
        let line = format!("未完了 {pending} / 失敗 {failed} / 要対応 {action} / 保存済み {saved}");
        if failed + action > 0 {
            r.add(Level::Warn, line, Some("メニューバーの履歴から再試行・対応してください"));
        } else {
            r.ok(line);
        }
        let last: Option<(Option<String>, Option<String>, String)> = conn
            .query_row(
                "SELECT last_error_code, last_error_message, status FROM jobs \
                 WHERE status IN ('error','action_required') ORDER BY updated_at DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        if let Some((code, msg, status)) = last {
            let label = crate::jobs::JobStatus::parse(&status).map(|s| s.label_ja()).unwrap_or("失敗");
            r.ok(format!(
                "直近のエラー（{label}）: {}{}",
                msg.unwrap_or_else(|| "（メッセージなし）".into()),
                code.map(|c| format!(" [{c}]")).unwrap_or_default()
            ));
        }
        Ok(())
    })();
    if let Err(e) = res {
        r.add(Level::Warn, format!("ジョブDBを読み取れません: {e}"), Some("アプリを終了してから再実行してください"));
    }
}

fn misc_section(r: &mut Report) {
    r.section("その他");
    if crate::install::is_launch_at_login() {
        r.ok("ログイン時に起動: 有効（LaunchAgent あり）");
    } else {
        r.add(
            Level::Warn,
            "ログイン時に起動: 無効（LaunchAgent なし）",
            Some("設定 > 一般 で「ログイン時に起動」をオンにしてください"),
        );
    }
    let log = support_dir().join("logs/holdnote.log");
    match std::fs::metadata(&log) {
        Ok(m) => r.ok(format!("ログ: {}（{}）", log.display(), human_size(m.len()))),
        Err(_) => r.ok(format!("ログ: {}（まだありません）", log.display())),
    }
    r.ok(format!("データフォルダ: {}", support_dir().display()));
}

pub fn run() {
    let mut r = Report::new();
    app_section(&mut r);
    let settings = settings_section(&mut r);
    mic_section(&mut r);
    whisper_section(&mut r, &settings);
    drive_section(&mut r, &settings);
    jobs_section(&mut r);
    misc_section(&mut r);
    println!("HoldNote 診断レポート\n");
    for l in &r.lines {
        println!("{l}");
    }
    println!("\n{}", summary_line(r.bads, r.warns));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_client_id() {
        assert_eq!(
            mask_client_id("123456789-abc.apps.googleusercontent.com"),
            "123456….apps.googleusercontent.com"
        );
        assert_eq!(mask_client_id("abcdefghij"), "abcdef…");
        assert_eq!(mask_client_id("ab"), "ab…");
    }

    #[test]
    fn detects_virtual_devices() {
        assert!(is_virtual_device("BlackHole 2ch"));
        assert!(is_virtual_device("Zoom Audio Device"));
        assert!(is_virtual_device("Aggregate Device"));
        assert!(is_virtual_device("Microsoft Teams Audio"));
        assert!(!is_virtual_device("MacBook Proのマイク"));
    }

    #[test]
    fn summary_lines() {
        assert_eq!(summary_line(0, 0), "問題は見つかりませんでした");
        assert_eq!(summary_line(2, 3), "2 件の問題、3 件の注意があります");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }
}
