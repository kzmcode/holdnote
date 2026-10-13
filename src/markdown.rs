//! Markdown file generation (requirements §5.2).
//! OWNER: transcriber/markdown agent.

use chrono::{DateTime, FixedOffset, SecondsFormat};

#[derive(Debug, Clone)]
pub struct MarkdownMeta {
    pub created_at: DateTime<FixedOffset>,
    pub duration_seconds: f64,
    pub language: String,
    /// "drive_api" in this build.
    pub save_mode: String,
}

/// `YYYY-MM-DD_HH-mm-ss_<first 6 hex chars of job id>.md`
pub fn file_name(created_at: &DateTime<FixedOffset>, job_id: &str) -> String {
    let short: String = job_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(6)
        .collect::<String>()
        .to_ascii_lowercase();
    format!("{}_{}.md", created_at.format("%Y-%m-%d_%H-%M-%S"), short)
}

/// Front matter + "# 音声メモ（YYYY-MM-DD HH:MM）" + "## 文字起こし" + transcript. Ends with "\n".
pub fn render(meta: &MarkdownMeta, transcript: &str) -> String {
    format!(
        "---\ncreated_at: {}\nduration_seconds: {:.1}\nlanguage: {}\nsource: HoldNote\ntranscription_engine: whisper.cpp\nsave_mode: {}\n---\n\n# 音声メモ（{}）\n\n## 文字起こし\n\n{}\n",
        meta.created_at.to_rfc3339_opts(SecondsFormat::Secs, false),
        meta.duration_seconds,
        meta.language,
        meta.save_mode,
        meta.created_at.format("%Y-%m-%d %H:%M"),
        transcript.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> MarkdownMeta {
        MarkdownMeta {
            created_at: DateTime::parse_from_rfc3339("2026-10-13T17:30:05+09:00").unwrap(),
            duration_seconds: 18.4,
            language: "ja".into(),
            save_mode: "local_sync".into(),
        }
    }

    #[test]
    fn name_matches_spec() {
        let m = meta();
        assert_eq!(
            file_name(&m.created_at, "A1B2C3d4-0000"),
            "2026-10-13_17-30-05_a1b2c3.md"
        );
    }

    #[test]
    fn render_matches_spec_example() {
        let out = render(&meta(), "この内容を来週の会議で共有したい。\n");
        let expected = "---\ncreated_at: 2026-10-13T17:30:05+09:00\nduration_seconds: 18.4\nlanguage: ja\nsource: HoldNote\ntranscription_engine: whisper.cpp\nsave_mode: local_sync\n---\n\n# 音声メモ（2026-10-13 17:30）\n\n## 文字起こし\n\nこの内容を来週の会議で共有したい。\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn duration_one_decimal() {
        let mut m = meta();
        m.duration_seconds = 3.0;
        assert!(render(&m, "x").contains("duration_seconds: 3.0\n"));
    }
}
