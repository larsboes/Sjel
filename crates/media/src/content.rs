//! Digest-bound content proposals; raw classifier evidence never becomes metadata implicitly.

use std::{collections::BTreeSet, fs::File, io::Read, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine};
use media_api::{
    ContentAction, ContentDecision, ContentDetail, ContentFrame, ContentPage, ContentProposal,
    ContentStageResult, ContentState,
};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    attachments::{validate_digest, Library},
    store::{Ledger, Result},
};

#[cfg(test)]
mod tests;

const ENGINE: &str = "apple-vision/VNClassifyImageRequest";
const MAX_REPORT_BYTES: u64 = 24 * 1024 * 1024;
const PAGE_SIZE: usize = 50;

pub(crate) fn migrate(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS media_content_proposals (
            id INTEGER PRIMARY KEY,
            sha256 TEXT NOT NULL REFERENCES media_attachments(sha256),
            fingerprint TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL CHECK(kind IN ('image','video')),
            engine TEXT NOT NULL, revision INTEGER NOT NULL, os TEXT NOT NULL,
            frames_json TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','accepted','rejected')),
            tags_json TEXT NOT NULL DEFAULT '[]', collections_json TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL DEFAULT ({}), decided_at TEXT
        );
        CREATE INDEX IF NOT EXISTS media_content_queue ON media_content_proposals(state,id);
        CREATE INDEX IF NOT EXISTS media_content_asset ON media_content_proposals(sha256,id);",
        sjel_store::NOW
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProbeReport {
    format_version: u32,
    engine: String,
    os: String,
    revision: u32,
    advisory: String,
    items: Vec<ProbeItem>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeItem {
    path: String,
    sha256: Option<String>,
    kind: Option<String>,
    frames: Vec<ContentFrame>,
    error: Option<String>,
}

fn visible(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn validate_frame(frame: &ContentFrame) -> Result<()> {
    if frame.labels.is_empty() || frame.labels.len() > 5 {
        return Err("each frame needs one to five candidate labels".into());
    }
    let mut labels = BTreeSet::new();
    for label in &frame.labels {
        if !visible(&label.identifier, 256)
            || !label.score.is_finite()
            || !(0.0..=1.0).contains(&label.score)
            || !labels.insert(&label.identifier)
        {
            return Err("invalid or repeated candidate label".into());
        }
    }
    if frame.preview_png.len() > 700_000 {
        return Err("preview exceeds the encoded size limit".into());
    }
    let png = STANDARD.decode(&frame.preview_png)?;
    if png.len() < 33
        || png.len() > 512 * 1024
        || &png[..8] != b"\x89PNG\r\n\x1a\n"
        || &png[12..16] != b"IHDR"
    {
        return Err("preview must be a bounded PNG".into());
    }
    let width = u32::from_be_bytes(png[16..20].try_into()?);
    let height = u32::from_be_bytes(png[20..24].try_into()?);
    if width == 0 || height == 0 || width > 256 || height > 256 {
        return Err("preview dimensions must be within 256 pixels".into());
    }
    Ok(())
}

fn validate_frames(kind: &str, frames: &[ContentFrame]) -> Result<()> {
    match kind {
        "image"
            if frames.len() == 1
                && frames[0].seconds.is_none()
                && frames[0].requested_seconds.is_none() => {}
        "video" if frames.len() == 3 => {
            let mut previous = -1.0;
            for frame in frames {
                let requested = frame
                    .requested_seconds
                    .ok_or("video frame lacks requested time")?;
                let actual = frame.seconds.ok_or("video frame lacks actual time")?;
                if !requested.is_finite()
                    || !actual.is_finite()
                    || requested <= 0.0
                    || actual < 0.0
                    || (requested - actual).abs() > 0.25
                    || actual <= previous
                {
                    return Err(
                        "video samples must be distinct, ordered and within 250 ms of requests"
                            .into(),
                    );
                }
                previous = actual;
            }
        }
        _ => return Err("invalid image/video frame coverage".into()),
    }
    for frame in frames {
        validate_frame(frame)?;
    }
    Ok(())
}

/// Stage a bounded native evidence report, verifying each successful digest through its owner.
///
/// # Errors
/// Invalid reports, unsupported evidence, missing/corrupt originals, or a database failure.
/// Any validation failure leaves the proposal table unchanged; native failed items are counted.
pub fn stage(ledger: &Ledger, library: &Library, path: &Path) -> Result<ContentStageResult> {
    let input = File::open(path)?;
    if !input.metadata()?.is_file() {
        return Err("report must be a regular file".into());
    }
    let mut bytes = Vec::new();
    input.take(MAX_REPORT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Err("report exceeds 24 MiB".into());
    }
    let report: ProbeReport = serde_json::from_slice(&bytes)?;
    if report.format_version != 1
        || report.engine != ENGINE
        || report.revision == 0
        || !visible(&report.os, 256)
        || report.items.is_empty()
        || report.items.len() > 20
        || report.advisory.len() > 4096
    {
        return Err("unsupported native evidence report".into());
    }
    let mut prepared = Vec::new();
    let mut result = ContentStageResult::default();
    let mut seen = BTreeSet::new();
    for item in report.items {
        // Source paths are not trusted or persisted; only the owner's verified digest is used.
        if item.path.len() > 4096 {
            return Err("source reference exceeds limit".into());
        }
        if item.error.is_some() {
            if item.sha256.is_some() || item.kind.is_some() || !item.frames.is_empty() {
                return Err("failed evidence must not contain successful results".into());
            }
            result.failed_items += 1;
            continue;
        }
        let digest = item.sha256.ok_or("successful item lacks digest")?;
        validate_digest(&digest)?;
        if !seen.insert(digest.clone()) {
            return Err("report repeats an attachment digest".into());
        }
        let kind = item.kind.ok_or("successful item lacks media kind")?;
        validate_frames(&kind, &item.frames)?;
        library
            .holding(&digest)?
            .ok_or("evidence original is not a local attachment")?;
        let canonical = serde_json::to_vec(&(
            &digest,
            &kind,
            &report.engine,
            report.revision,
            &report.os,
            &item.frames,
        ))?;
        let fingerprint = crate::hex_digest(&Sha256::digest(canonical));
        prepared.push((
            digest,
            kind,
            fingerprint,
            serde_json::to_string(&item.frames)?,
        ));
    }
    let mut conn = ledger.pool.get()?;
    let tx = sjel_store::write_transaction(&mut conn)?;
    for (digest, kind, fingerprint, frames) in prepared {
        let inserted = tx.execute(
            "INSERT INTO media_content_proposals(sha256,fingerprint,kind,engine,revision,os,frames_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(fingerprint) DO NOTHING",
            params![digest, fingerprint, kind, report.engine, report.revision, report.os, frames],
        )?;
        result.inserted += inserted;
        result.unchanged += usize::from(inserted == 0);
    }
    tx.commit()?;
    Ok(result)
}

const COLUMNS: &str =
    "id,sha256,fingerprint,kind,engine,revision,state,tags_json,collections_json,created_at";
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContentProposal> {
    let parse = |index| -> rusqlite::Result<Vec<String>> {
        serde_json::from_str(&row.get::<_, String>(index)?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
    };
    let state = match row.get::<_, String>(6)?.as_str() {
        "pending" => ContentState::Pending,
        "accepted" => ContentState::Accepted,
        "rejected" => ContentState::Rejected,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(ContentProposal {
        id: row.get(0)?,
        sha256: row.get(1)?,
        fingerprint: row.get(2)?,
        kind: row.get(3)?,
        engine: row.get(4)?,
        revision: row.get(5)?,
        state,
        tags: parse(7)?,
        collections: parse(8)?,
        created_at: row.get(9)?,
    })
}

/// Read a bounded queue page; `all` includes explicit decisions for later inspection.
///
/// # Errors
/// Invalid filters/cursors, malformed stored metadata, or a database failure.
pub fn list(ledger: &Ledger, state: &str, after: i64) -> Result<ContentPage> {
    if !["pending", "accepted", "rejected", "all"].contains(&state) || after < 0 {
        return Err("invalid content queue state or cursor".into());
    }
    let conn = ledger.pool.get()?;
    let mut query = conn.prepare(&format!("SELECT {COLUMNS} FROM media_content_proposals WHERE (?1='all' OR state=?1) AND id>?2 ORDER BY id LIMIT ?3"))?;
    let mut items = query
        .query_map(params![state, after, (PAGE_SIZE + 1) as i64], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let next_after = if items.len() > PAGE_SIZE {
        items.truncate(PAGE_SIZE);
        items.last().map(|p| p.id)
    } else {
        None
    };
    Ok(ContentPage { items, next_after })
}

/// Read one proposal's bounded evidence, without reading arbitrary source paths.
///
/// # Errors
/// Missing proposals, corrupt evidence JSON, or a database failure.
pub fn detail(ledger: &Ledger, id: i64) -> Result<Option<ContentDetail>> {
    let conn = ledger.pool.get()?;
    let proposal = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM media_content_proposals WHERE id=?1"),
            [id],
            row,
        )
        .optional()?;
    let Some(proposal) = proposal else {
        return Ok(None);
    };
    let (os, frames): (String, String) = conn.query_row(
        "SELECT os,frames_json FROM media_content_proposals WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut names = conn.prepare(
        "SELECT filename FROM media_attachment_sources WHERE sha256=?1 ORDER BY filename",
    )?;
    let filenames = names
        .query_map([&proposal.sha256], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(ContentDetail {
        proposal,
        os,
        frames: serde_json::from_str(&frames)?,
        filenames,
    }))
}

/// Expected review refusals, distinct from storage and original-verification failures.
#[derive(Debug)]
pub enum DecisionError {
    Invalid(String),
    NotFound,
    Conflict(&'static str),
}

impl std::fmt::Display for DecisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::NotFound => f.write_str("content proposal not found"),
            Self::Conflict(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DecisionError {}

fn normalized(values: &[String]) -> std::result::Result<Vec<String>, DecisionError> {
    if values.len() > 32 || values.iter().any(|v| !visible(v, 128)) {
        return Err(DecisionError::Invalid(
            "choose at most 32 visible metadata names of at most 128 bytes".into(),
        ));
    }
    Ok(values
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

/// Persist an exact-evidence decision; retries are idempotent and conflicting decisions refuse.
///
/// # Errors
/// Invalid assignments, stale fingerprints, conflicting decisions, or corrupt/missing originals.
pub fn decide(
    ledger: &Ledger,
    library: &Library,
    id: i64,
    decision: &ContentDecision,
) -> Result<ContentProposal> {
    validate_digest(&decision.fingerprint).map_err(|e| DecisionError::Invalid(e.to_string()))?;
    let tags = normalized(&decision.tags)?;
    let collections = normalized(&decision.collections)?;
    let state = match decision.action {
        ContentAction::Accept if !tags.is_empty() || !collections.is_empty() => "accepted",
        ContentAction::Reject if tags.is_empty() && collections.is_empty() => "rejected",
        _ => {
            return Err(DecisionError::Invalid(
                "accept needs chosen metadata; reject cannot carry metadata".into(),
            )
            .into())
        }
    };
    let held = detail(ledger, id)?.ok_or(DecisionError::NotFound)?;
    if held.proposal.fingerprint != decision.fingerprint {
        return Err(DecisionError::Conflict("content evidence changed; refresh review").into());
    }
    if decision.action == ContentAction::Accept {
        library
            .holding(&held.proposal.sha256)?
            .ok_or("attachment original is missing")?;
    }
    let mut conn = ledger.pool.get()?;
    let tx = sjel_store::write_transaction(&mut conn)?;
    tx.execute(&format!("UPDATE media_content_proposals SET state=?1,tags_json=?2,collections_json=?3,decided_at={} WHERE id=?4 AND fingerprint=?5 AND state='pending'", sjel_store::NOW),
        params![state, serde_json::to_string(&tags)?, serde_json::to_string(&collections)?, id, decision.fingerprint])?;
    let saved = tx.query_row(
        &format!("SELECT {COLUMNS} FROM media_content_proposals WHERE id=?1"),
        [id],
        row,
    )?;
    let wanted = if decision.action == ContentAction::Accept {
        ContentState::Accepted
    } else {
        ContentState::Rejected
    };
    if saved.state != wanted || saved.tags != tags || saved.collections != collections {
        return Err(DecisionError::Conflict(
            "proposal already decided differently; refresh review",
        )
        .into());
    }
    tx.commit()?;
    Ok(saved)
}
