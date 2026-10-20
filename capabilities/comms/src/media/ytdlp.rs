//! The yt-dlp rung: everything that reaches a video's metadata and subtitles
//! through a subprocess.
//!
//! Its own file because it is the one adapter that fetches outside the shared
//! client -- a subprocess the redirect policy never sees -- so `media::fetch`
//! checks the destination before it gets here and nothing below re-checks it.

use super::*;

/// A temp directory removed on drop (covers every early-return / error path).
pub(super) struct TmpDir(PathBuf);

impl TmpDir {
    pub(super) fn new(tag: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!("comms-ingest-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Deserialize, Default)]
pub(super) struct YtMeta {
    pub(super) title: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    uploader_id: Option<String>,
    // duration is parsed but unused today beyond presence; kept for the record.
    #[allow(dead_code)]
    duration: Option<f64>,
}

/// `yt-dlp --dump-json --skip-download <url>` -> parsed metadata.
/// Args for the `--dump-json` metadata call. `impersonate` adds the browser
/// impersonation flags YouTube's anti-bot needs under load.
pub(super) fn ytdlp_meta_args(url: &str, impersonate: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["--dump-json".into(), "--skip-download".into()];
    if impersonate {
        a.push("--impersonate".into());
        a.push("chrome".into());
    }
    a.push(url.to_string());
    a
}

/// Args for the subtitle fetch. `impersonate` as above.
pub(super) fn ytdlp_sub_args(url: &str, dir: &Path, impersonate: bool) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "--skip-download".into(),
        // Without --ignore-errors, yt-dlp stops after the first missing
        // language. Prefer English, then retain German as a multilingual-input
        // fallback; generated Sjel text still defaults to English.
        "--ignore-errors".into(),
        "--write-auto-subs".into(),
        "--write-subs".into(),
        "--sub-langs".into(),
        "en,en-orig,de".into(),
        "--sub-format".into(),
        "vtt/best".into(),
    ];
    if impersonate {
        a.push("--impersonate".into());
        a.push("chrome".into());
    }
    a.push("-P".into());
    a.push(dir.to_string_lossy().into_owned());
    a.push("-o".into());
    a.push("sub".into());
    a.push(url.to_string());
    a
}

/// Run yt-dlp trying browser impersonation first (YouTube 429s the subtitle/
/// metadata endpoints otherwise). If that run fails *because the impersonate
/// target is unavailable on this machine* (stderr mentions "impersonate"),
/// retry once without the flag. Any other failure returns None -- the caller
/// keeps its existing degrade behavior. `make_args(impersonate)` builds the arg
/// list for each attempt.
pub(super) fn run_ytdlp_with_fallback(
    make_args: impl Fn(bool) -> Vec<String>,
) -> Option<std::process::Output> {
    let first = Command::new("yt-dlp").args(make_args(true)).output().ok()?;
    if first.status.success() {
        return Some(first);
    }
    if String::from_utf8_lossy(&first.stderr).contains("impersonate") {
        let second = Command::new("yt-dlp")
            .args(make_args(false))
            .output()
            .ok()?;
        if second.status.success() {
            return Some(second);
        }
    }
    None
}

pub(super) fn ytdlp_meta(url: &str) -> Result<YtMeta> {
    let out = run_ytdlp_with_fallback(|imp| ytdlp_meta_args(url, imp)).ok_or_else(|| {
        CommsError::Other("yt-dlp metadata failed (not runnable or fetch error)".into())
    })?;
    // --dump-json emits one JSON object per line; take the first.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().unwrap_or("{}");
    Ok(serde_json::from_str(line).unwrap_or_default())
}

pub(super) fn author_of(m: &YtMeta) -> Option<String> {
    m.uploader
        .clone()
        .or_else(|| m.channel.clone())
        .or_else(|| m.uploader_id.clone())
}

/// Download subtitles into `dir` and return the parsed transcript, if any.
/// Never fails the ingest -- returns None on any subtitle error.
pub(super) fn ytdlp_transcript(url: &str, dir: &Path) -> Option<String> {
    run_ytdlp_with_fallback(|imp| ytdlp_sub_args(url, dir, imp))?;
    let vtt = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().map(|x| x == "vtt").unwrap_or(false))?;
    let body = std::fs::read_to_string(&vtt).ok()?;
    let text = parse_vtt(&body);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Strip a VTT into plain transcript text: drop the header, cue numbers,
/// timestamp lines, inline `<...>` tags, and consecutive duplicate lines.
pub(super) fn parse_vtt(body: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line == "WEBVTT"
            || line.starts_with("Kind:")
            || line.starts_with("Language:")
            || line.starts_with("NOTE")
            || line.contains("-->")
            || line.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        let cleaned = strip_tags(line);
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        if lines.last().map(|l| l == cleaned).unwrap_or(false) {
            continue; // dedupe consecutive duplicates (common in auto-subs)
        }
        lines.push(cleaned.to_string());
    }
    lines.join("\n")
}
