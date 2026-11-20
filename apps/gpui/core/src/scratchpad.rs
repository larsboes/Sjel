//! The 24-hour quick scratchpad (`sjel://scratchpad`): a transient daily notepad for thoughts,
//! links, and tasks that persists across restarts for 24 hours and automatically sorts into Sjel.
//!
//! Line classification rules:
//! - Tasks (`- [ ] ...`, `* [ ] ...`, `[] ...`, or `#task` / `#todo`): routed to Projects tasks
//!   (in a `@ProjectName` project if tagged, otherwise `"Daily"` or the first active project).
//! - Done tasks (`- [x] ...`): skipped or archived into the daily note.
//! - Purchase items (`#buy`): routed to Inventory wishlist (`CaptureRoute::Wishlist`).
//! - Places (`#go`): routed to Places (`CaptureRoute::Places`).
//! - Links (`http://` or `https://`, or `#look` / `#learn` with a URL): routed to Feed (`CaptureRoute::Feed`)
//!   or kept as a tagged capture in Inbox when no URL is present.
//! - Freeform notes: bundled into a dated note (`Scratchpad YYYY-MM-DD`) in Projects.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::capture::{self, Capture, Draft, Route};
use crate::data::Repo;
use crate::shell::{ShellError, ShellResult};

/// 24 hours in seconds.
pub const EXPIRY_SECS: u64 = 24 * 60 * 60;

/// What a single line or block in the scratchpad will sort into.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScratchTarget {
    /// A task in Projects (`project` is `Some` when `@Project` is specified, else default).
    Task {
        title: String,
        project: Option<String>,
        done: bool,
    },
    /// A wishlist item in Inventory (`#buy`).
    Wishlist {
        text: Option<String>,
        url: Option<String>,
    },
    /// A place in Places (`#go`).
    Place {
        text: Option<String>,
        url: Option<String>,
    },
    /// A link in Feed (`#look`, `#learn` with URL, or bare/annotated URL).
    Feed {
        text: Option<String>,
        url: String,
        tag: Option<String>,
    },
    /// A capture in Inbox (e.g. `#learn` without a URL).
    Inbox { text: String, tag: Option<String> },
    /// A freeform note line that joins the daily note in Projects.
    Note {
        text: String,
        project: Option<String>,
    },
}

impl ScratchTarget {
    pub fn badge_label(&self) -> &'static str {
        match self {
            Self::Task { done: true, .. } => "Done task",
            Self::Task { done: false, .. } => "→ Tasks",
            Self::Wishlist { .. } => "→ Wishlist",
            Self::Place { .. } => "→ Places",
            Self::Feed { .. } => "→ Feed",
            Self::Inbox { .. } => "→ Inbox",
            Self::Note { .. } => "→ Daily Note",
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Task {
                title,
                project: Some(p),
                ..
            } => format!("{title} (@{p})"),
            Self::Task {
                title,
                project: None,
                ..
            } => title.clone(),
            Self::Wishlist { text, url } | Self::Place { text, url } => text
                .clone()
                .filter(|t| !t.is_empty())
                .or_else(|| capture::host(url.as_deref()))
                .or_else(|| url.clone())
                .unwrap_or_default(),
            Self::Feed { text, url, .. } => text
                .clone()
                .filter(|t| !t.is_empty())
                .or_else(|| capture::host(Some(url.as_str())))
                .unwrap_or_else(|| url.clone()),
            Self::Inbox { text, .. } | Self::Note { text, .. } => text.clone(),
        }
    }
}

/// Summary of a completed sort operation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortOutcome {
    pub sorted_at: u64,
    pub date_label: String,
    pub tasks_created: usize,
    pub wishlist_created: usize,
    pub places_created: usize,
    pub feed_routed: usize,
    pub inbox_captured: usize,
    pub note_lines_saved: usize,
    pub note_path: Option<String>,
}

impl SortOutcome {
    pub fn total_items(&self) -> usize {
        self.tasks_created
            + self.wishlist_created
            + self.places_created
            + self.feed_routed
            + self.inbox_captured
            + usize::from(self.note_lines_saved > 0)
    }

    pub fn summary_line(&self) -> String {
        let mut parts = Vec::new();
        if self.tasks_created > 0 {
            parts.push(format!(
                "{} {}",
                self.tasks_created,
                if self.tasks_created == 1 {
                    "task"
                } else {
                    "tasks"
                }
            ));
        }
        if self.feed_routed > 0 {
            parts.push(format!(
                "{} {}",
                self.feed_routed,
                if self.feed_routed == 1 {
                    "link"
                } else {
                    "links"
                }
            ));
        }
        if self.wishlist_created > 0 {
            parts.push(format!("{} wishlist", self.wishlist_created));
        }
        if self.places_created > 0 {
            parts.push(format!(
                "{} {}",
                self.places_created,
                if self.places_created == 1 {
                    "place"
                } else {
                    "places"
                }
            ));
        }
        if self.inbox_captured > 0 {
            parts.push(format!("{} inbox", self.inbox_captured));
        }
        if self.note_lines_saved > 0 {
            parts.push("daily note".to_owned());
        }
        if parts.is_empty() {
            "Nothing to sort".to_owned()
        } else {
            format!("Sorted {}", parts.join(" · "))
        }
    }
}

/// Persisted state of the 24-hour scratchpad (`~/.config/sjel/scratchpad.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScratchpadState {
    /// Raw markdown/text content of the scratchpad.
    pub text: String,
    /// Unix timestamp (seconds) when the current scratchpad session started (first non-empty edit).
    pub started_at: Option<u64>,
    /// Unix timestamp (seconds) of the last edit.
    pub updated_at: Option<u64>,
    /// Optional default project name for tasks and daily notes.
    pub target_project: Option<String>,
    /// Recent sort outcomes (kept up to 5) so the user can see where yesterday's scratchpad went.
    pub history: Vec<SortOutcome>,
}

impl ScratchpadState {
    pub fn config_path() -> Option<PathBuf> {
        if let Ok(dir) = std::env::var("SJEL_CONFIG_DIR") {
            return Some(PathBuf::from(dir).join("scratchpad.json"));
        }
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join("sjel/scratchpad.json"));
        }
        if let Ok(home) = std::env::var("HOME") {
            return Some(PathBuf::from(home).join(".config/sjel/scratchpad.json"));
        }
        None
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Self>(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Err("Could not determine scratchpad config path".into());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create config dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Scratchpad serialize error: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("Scratchpad write error: {e}"))
    }

    /// Updates the scratchpad text and timestamps. Resets `started_at` when cleared.
    pub fn set_text(&mut self, next: String, now_secs: u64) {
        if next.trim().is_empty() {
            self.text.clear();
            self.started_at = None;
            self.updated_at = Some(now_secs);
            return;
        }
        if self.started_at.is_none() || self.text.trim().is_empty() {
            self.started_at = Some(now_secs);
        }
        self.text = next;
        self.updated_at = Some(now_secs);
    }

    /// Appends a single quick-capture line to the scratchpad.
    pub fn append_line(&mut self, line: &str, now_secs: u64) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        let next = if self.text.trim().is_empty() {
            trimmed.to_owned()
        } else if self.text.ends_with('\n') {
            format!("{}{trimmed}", self.text)
        } else {
            format!("{}\n{trimmed}", self.text)
        };
        self.set_text(next, now_secs);
    }

    /// Seconds remaining before the 24-hour window expires, or `None` if empty.
    pub fn remaining_secs(&self, now_secs: u64) -> Option<u64> {
        if self.text.trim().is_empty() {
            return None;
        }
        let started = self.started_at.unwrap_or(now_secs);
        let expires_at = started.saturating_add(EXPIRY_SECS);
        Some(expires_at.saturating_sub(now_secs))
    }

    /// True when the scratchpad has non-empty content and its 24-hour window has elapsed.
    pub fn is_expired(&self, now_secs: u64) -> bool {
        self.remaining_secs(now_secs) == Some(0)
    }

    /// Human-readable countdown badge (`"23h 15m left"`, `"42m left"`, `"Sorting due"`).
    pub fn countdown_label(&self, now_secs: u64) -> Option<String> {
        let rem = self.remaining_secs(now_secs)?;
        if rem == 0 {
            return Some("24h elapsed · Ready to sort".to_owned());
        }
        let hours = rem / 3600;
        let mins = (rem % 3600) / 60;
        if hours > 0 {
            Some(format!("{hours}h {mins}m left"))
        } else {
            Some(format!("{}m left", mins.max(1)))
        }
    }

    /// Parses the current text into classified targets in order.
    pub fn parse_targets(&self) -> Vec<ScratchTarget> {
        parse_scratchpad(&self.text, self.target_project.as_deref())
    }
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Extracts an optional `@ProjectName` token from a line, returning `(project, remaining_line)`.
fn extract_project_mention(line: &str) -> (Option<String>, String) {
    let mut project = None;
    let mut kept = Vec::new();
    for word in line.split_whitespace() {
        if project.is_none() {
            if let Some(name) = word.strip_prefix('@') {
                let clean =
                    name.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
                if !clean.is_empty() {
                    project = Some(clean.to_owned());
                    continue;
                }
            }
        }
        kept.push(word);
    }
    (project, kept.join(" "))
}

/// Parses a single line of scratchpad text into a [`ScratchTarget`], or `None` if blank.
pub fn parse_line(line: &str, default_project: Option<&str>) -> Option<ScratchTarget> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 1. Check for markdown task checkboxes: `- [ ]`, `* [ ]`, `[ ]`, `- [x]`, `[x]`
    let task_prefix = [
        ("- [ ] ", false),
        ("* [ ] ", false),
        ("[ ] ", false),
        ("[] ", false),
        ("- [x] ", true),
        ("- [X] ", true),
        ("* [x] ", true),
        ("[x] ", true),
        ("[X] ", true),
    ];
    for (prefix, done) in task_prefix {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let (proj, title) = extract_project_mention(rest);
            let title = title.trim().to_owned();
            if !title.is_empty() {
                return Some(ScratchTarget::Task {
                    title,
                    project: proj.or_else(|| default_project.map(str::to_owned)),
                    done,
                });
            }
        }
    }

    // 2. Check for `#task` or `#todo` hashtag anywhere in the line
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let has_task_tag = words
        .iter()
        .any(|w| matches!(w.to_ascii_lowercase().as_str(), "#task" | "#todo"));
    if has_task_tag {
        let stripped = words
            .into_iter()
            .filter(|w| !matches!(w.to_ascii_lowercase().as_str(), "#task" | "#todo"))
            .collect::<Vec<_>>()
            .join(" ");
        let (proj, title) = extract_project_mention(&stripped);
        let title = title.trim().to_owned();
        if !title.is_empty() {
            return Some(ScratchTarget::Task {
                title,
                project: proj.or_else(|| default_project.map(str::to_owned)),
                done: false,
            });
        }
    }

    // 3. Check for URLs or capture hashtags (`#buy`, `#look`, `#go`, `#learn`)
    let url = trimmed
        .split_whitespace()
        .find(|w| w.starts_with("http://") || w.starts_with("https://"))
        .map(str::to_owned);
    let without_url = match &url {
        Some(u) => trimmed
            .split_whitespace()
            .filter(|w| *w != u)
            .collect::<Vec<_>>()
            .join(" "),
        None => trimmed.to_owned(),
    };
    let parse_input = if without_url.trim().is_empty() {
        trimmed
    } else {
        without_url.as_str()
    };

    if let Ok(mut draft) = Draft::parse(parse_input, None) {
        if draft.url.is_none() {
            draft.url = url.clone();
        }
        if let Some(route) = draft.route() {
            return Some(match route {
                Route::Wishlist => ScratchTarget::Wishlist {
                    text: draft.text,
                    url: draft.url,
                },
                Route::Places => ScratchTarget::Place {
                    text: draft.text,
                    url: draft.url,
                },
                Route::Feed => {
                    let tag = draft
                        .tag
                        .clone()
                        .or_else(|| draft.suggested_tag().map(str::to_owned));
                    let target_url = draft.url.unwrap_or_default();
                    ScratchTarget::Feed {
                        text: draft.text,
                        url: target_url,
                        tag,
                    }
                }
            });
        }
        if draft.tag.is_some() {
            if let Some(u) = draft.url {
                return Some(ScratchTarget::Feed {
                    text: draft.text,
                    url: u,
                    tag: draft.tag,
                });
            }
            return Some(ScratchTarget::Inbox {
                text: draft.text.unwrap_or_else(|| trimmed.to_owned()),
                tag: draft.tag,
            });
        }
    }

    // Any line containing an `http(s)://` link without an explicit tag routes to Feed (or Inbox capture + Feed)
    if let Some(u) = url {
        let note_text = trimmed
            .split_whitespace()
            .filter(|w| *w != u)
            .collect::<Vec<_>>()
            .join(" ");
        return Some(ScratchTarget::Feed {
            text: (!note_text.is_empty()).then_some(note_text),
            url: u,
            tag: Some("look".to_owned()),
        });
    }

    // 4. Otherwise it is a freeform note line
    let (proj, clean) = extract_project_mention(trimmed);
    let text = if proj.is_some() && !clean.is_empty() {
        clean
    } else {
        trimmed.to_owned()
    };
    Some(ScratchTarget::Note {
        text,
        project: proj.or_else(|| default_project.map(str::to_owned)),
    })
}

/// Parses an entire scratchpad document into [`ScratchTarget`] items.
pub fn parse_scratchpad(text: &str, default_project: Option<&str>) -> Vec<ScratchTarget> {
    text.lines()
        .filter_map(|line| parse_line(line, default_project))
        .collect()
}

/// Executes the sorting of all items in `targets` against the live Sjel capabilities via `Repo`.
pub async fn sort_into_sjel(
    repo: &Repo,
    targets: Vec<ScratchTarget>,
    date_label: &str,
    now_secs: u64,
) -> ShellResult<SortOutcome> {
    let mut outcome = SortOutcome {
        sorted_at: now_secs,
        date_label: date_label.to_owned(),
        ..SortOutcome::default()
    };
    if targets.is_empty() {
        return Ok(outcome);
    }

    // Resolve the fallback project name for tasks and daily notes.
    let projects = repo.fetch_projects().await.unwrap_or_default();
    let fallback_project = projects
        .iter()
        .find(|p| !p.archived && p.name.eq_ignore_ascii_case("Daily"))
        .or_else(|| {
            projects
                .iter()
                .find(|p| !p.archived && p.name.eq_ignore_ascii_case("Personal"))
        })
        .or_else(|| projects.iter().find(|p| !p.archived))
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Daily".to_owned());

    let mut ensure_daily_project = !projects.iter().any(|p| p.name == fallback_project);
    let mut note_lines_by_project: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

    for target in targets {
        match target {
            ScratchTarget::Task {
                title,
                project,
                done,
            } => {
                let proj = project.unwrap_or_else(|| fallback_project.clone());
                if proj == fallback_project && ensure_daily_project {
                    let _ = repo
                        .add_project(&fallback_project, "Daily notes and scratchpad tasks")
                        .await;
                    ensure_daily_project = false;
                }
                if done {
                    note_lines_by_project
                        .entry(proj)
                        .or_default()
                        .push(format!("- [x] {title}"));
                } else {
                    match repo.add_task(&proj, &title).await {
                        Ok(()) => outcome.tasks_created += 1,
                        Err(_) => {
                            // If task creation failed (e.g. duplicate title or external task source),
                            // preserve it in the daily note so nothing is ever lost.
                            note_lines_by_project
                                .entry(proj)
                                .or_default()
                                .push(format!("- [ ] {title}"));
                        }
                    }
                }
            }
            ScratchTarget::Wishlist { text, url } => {
                if let Ok(shell) = repo.shell() {
                    let mut raw = text.clone().unwrap_or_default();
                    if !raw.contains("#buy") {
                        raw = format!("#buy {raw}").trim().to_owned();
                    }
                    if let Ok(cap) = shell.add_capture(Some(&raw), url.as_deref()).await {
                        if shell.route_capture(&cap, Route::Wishlist).await.is_ok() {
                            outcome.wishlist_created += 1;
                            continue;
                        }
                    }
                }
                // Fallback to daily note if shell is offline
                let line = match (text, url) {
                    (Some(t), Some(u)) => format!("- #buy {t} ({u})"),
                    (Some(t), None) => format!("- #buy {t}"),
                    (None, Some(u)) => format!("- #buy {u}"),
                    (None, None) => continue,
                };
                note_lines_by_project
                    .entry(fallback_project.clone())
                    .or_default()
                    .push(line);
            }
            ScratchTarget::Place { text, url } => {
                if let Ok(shell) = repo.shell() {
                    let mut raw = text.clone().unwrap_or_default();
                    if !raw.contains("#go") {
                        raw = format!("#go {raw}").trim().to_owned();
                    }
                    if let Ok(cap) = shell.add_capture(Some(&raw), url.as_deref()).await {
                        if shell.route_capture(&cap, Route::Places).await.is_ok() {
                            outcome.places_created += 1;
                            continue;
                        }
                    }
                }
                let line = match (text, url) {
                    (Some(t), Some(u)) => format!("- #go {t} ({u})"),
                    (Some(t), None) => format!("- #go {t}"),
                    (None, Some(u)) => format!("- #go {u}"),
                    (None, None) => continue,
                };
                note_lines_by_project
                    .entry(fallback_project.clone())
                    .or_default()
                    .push(line);
            }
            ScratchTarget::Feed { text, url, tag } => {
                if let Ok(shell) = repo.shell() {
                    let prefixed = match (&tag, &text) {
                        (Some(tag), Some(t)) => Some(format!("#{tag} {t}")),
                        (Some(tag), None) => Some(format!("#{tag}")),
                        (None, Some(t)) => Some(t.clone()),
                        (None, None) => None,
                    };
                    if let Ok(cap) = shell
                        .add_capture(prefixed.as_deref(), Some(url.as_str()))
                        .await
                    {
                        if shell.route_capture(&cap, Route::Feed).await.is_ok() {
                            outcome.feed_routed += 1;
                            continue;
                        } else {
                            // Kept in inbox if feed ingest required approval or failed
                            outcome.inbox_captured += 1;
                            continue;
                        }
                    } else {
                        // Try direct capture without route
                        let fake = Capture {
                            id: "0000000000000000".into(),
                            text: text.clone(),
                            url: Some(url.clone()),
                            tag,
                            status: "new".into(),
                            routed_to: None,
                            client: Some("scratchpad".into()),
                            created_at: date_label.to_owned(),
                        };
                        let _ = fake;
                    }
                }
                let line = match text {
                    Some(t) if !t.is_empty() => format!("- [{t}]({url})"),
                    _ => format!("- {url}"),
                };
                note_lines_by_project
                    .entry(fallback_project.clone())
                    .or_default()
                    .push(line);
            }
            ScratchTarget::Inbox { text, tag } => {
                if let Ok(shell) = repo.shell() {
                    let raw = match tag {
                        Some(t) => format!("#{t} {text}"),
                        None => text.clone(),
                    };
                    if shell.add_capture(Some(&raw), None).await.is_ok() {
                        outcome.inbox_captured += 1;
                        continue;
                    }
                }
                note_lines_by_project
                    .entry(fallback_project.clone())
                    .or_default()
                    .push(text);
            }
            ScratchTarget::Note { text, project } => {
                let proj = project.unwrap_or_else(|| fallback_project.clone());
                note_lines_by_project.entry(proj).or_default().push(text);
            }
        }
    }

    // Write accumulated freeform note lines into each target project's dated scratchpad note.
    for (proj, lines) in note_lines_by_project {
        if lines.is_empty() {
            continue;
        }
        if proj == fallback_project && ensure_daily_project {
            let _ = repo
                .add_project(&fallback_project, "Daily notes and scratchpad tasks")
                .await;
            ensure_daily_project = false;
        }
        let title = format!("Scratchpad {date_label}");
        let path = format!("{proj}/{title}.md");
        let block = lines.join("\n");

        // Append if the dated note already exists today, otherwise create it and save the body.
        let existing = repo.fetch_record(&path).await.ok();
        let saved = if let Some(record) = existing {
            let updated = format!("{}\n\n{}", record.content.trim_end(), block);
            repo.save_record(&path, &updated, &record.content).await
        } else {
            let _ = repo.add_note(&proj, &title).await;
            match repo.fetch_record(&path).await {
                Ok(record) => {
                    let updated = format!("{}\n\n{}", record.content.trim_end(), block);
                    repo.save_record(&path, &updated, &record.content).await
                }
                Err(e) => Err(e),
            }
        };

        match saved {
            Ok(()) => {
                outcome.note_lines_saved += lines.len();
                if outcome.note_path.is_none() {
                    outcome.note_path = Some(path);
                }
            }
            Err(err) => {
                // Fallback: save to Inbox capture so nothing is lost even if Projects is unreachable
                if let Ok(shell) = repo.shell() {
                    if shell.add_capture(Some(&block), None).await.is_ok() {
                        outcome.inbox_captured += 1;
                        continue;
                    }
                }
                if outcome.total_items() == 0 {
                    return Err(ShellError::Store(format!(
                        "Could not sort scratchpad into Sjel: {err}"
                    )));
                }
            }
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tasks_links_tags_and_freeform_notes() {
        let input = "\
- [ ] Call the dentist @Personal
#buy Linen shirt 49,90 € https://shop.example/shirt
https://arxiv.org/abs/2611.12345 Paper on attention
#go Lisbon cafe
#task Refactor header navigation @Sjel
Remember to measure frame times before committing
";
        let targets = parse_scratchpad(input, None);
        assert_eq!(targets.len(), 6);
        assert_eq!(
            targets[0],
            ScratchTarget::Task {
                title: "Call the dentist".into(),
                project: Some("Personal".into()),
                done: false,
            }
        );
        assert_eq!(
            targets[1],
            ScratchTarget::Wishlist {
                text: Some("Linen shirt 49,90 €".into()),
                url: Some("https://shop.example/shirt".into()),
            }
        );
        assert_eq!(
            targets[2],
            ScratchTarget::Feed {
                text: Some("Paper on attention".into()),
                url: "https://arxiv.org/abs/2611.12345".into(),
                tag: Some("look".into()),
            }
        );
        assert_eq!(
            targets[3],
            ScratchTarget::Place {
                text: Some("Lisbon cafe".into()),
                url: None,
            }
        );
        assert_eq!(
            targets[4],
            ScratchTarget::Task {
                title: "Refactor header navigation".into(),
                project: Some("Sjel".into()),
                done: false,
            }
        );
        assert_eq!(
            targets[5],
            ScratchTarget::Note {
                text: "Remember to measure frame times before committing".into(),
                project: None,
            }
        );
    }

    #[test]
    fn tracks_24h_expiry_and_resets_when_cleared() {
        let mut state = ScratchpadState::default();
        assert_eq!(state.remaining_secs(1_000), None);
        assert!(!state.is_expired(1_000));

        state.set_text("Quick thought".into(), 1_000);
        assert_eq!(state.started_at, Some(1_000));
        assert_eq!(state.remaining_secs(1_000), Some(EXPIRY_SECS));
        assert_eq!(
            state.countdown_label(1_000 + 3600).as_deref(),
            Some("23h 0m left")
        );
        assert!(!state.is_expired(1_000 + EXPIRY_SECS - 1));
        assert!(state.is_expired(1_000 + EXPIRY_SECS));

        state.set_text("   ".into(), 2_000);
        assert_eq!(state.started_at, None);
        assert_eq!(state.remaining_secs(2_000), None);
    }
}
