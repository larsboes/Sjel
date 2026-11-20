use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::AppState;
use crate::store::{item_snapshot, Store};

type Error = (StatusCode, String);
fn internal(e: impl std::fmt::Display) -> Error {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
fn invalid(message: &str) -> Error {
    (StatusCode::BAD_REQUEST, message.into())
}
fn conflict(message: &str) -> Error {
    (StatusCode::CONFLICT, message.into())
}

pub use inventory_api::mail_evidence::MailProposalInput as Stage;
use inventory_api::mail_evidence::PurchaseEvent;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub(crate) version: i64,
    pub(crate) action: String,
    pub(crate) title: Option<String>,
    pub(crate) item_id: Option<String>,
    pub(crate) expected_item_revision: Option<i64>,
    pub(crate) expected_state_id: Option<i64>,
}

#[derive(Serialize)]
pub(crate) struct Proposal {
    pub(crate) id: i64,
    #[serde(flatten)]
    pub(crate) stage: Stage,
    pub(crate) version: i64,
    pub(crate) decision: Option<String>,
    pub(crate) item_id: Option<String>,
    pub(crate) approved_title: Option<String>,
    pub(crate) matched_item_revision: Option<i64>,
    pub(crate) matched_state_id: Option<i64>,
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Proposal> {
    Ok(Proposal {
        id: row.get(0)?,
        stage: Stage {
            source_id: row.get(1)?,
            line_id: row.get(2)?,
            event: {
                let event: String = row.get(3)?;
                serde_json::from_value(serde_json::Value::String(event)).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?
            },
            title: row.get(4)?,
            evidence: row.get(5)?,
            data_class: row.get(6)?,
            order_ref: row.get(7)?,
            claim: row.get(14)?,
            extraction_version: row.get(15)?,
        },
        version: row.get(8)?,
        decision: row.get(9)?,
        item_id: row.get(10)?,
        approved_title: row.get(11)?,
        matched_item_revision: row.get(12)?,
        matched_state_id: row.get(13)?,
    })
}
pub(crate) const COLUMNS: &str = "id, source_id, line_id, event, title, evidence, data_class, order_ref, version, decision, item_id, approved_title, matched_item_revision, matched_state_id, claim, extraction_version";

fn bounded(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.contains('\0')
}
pub async fn stage(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Stage>,
) -> Result<Json<serde_json::Value>, Error> {
    body.validate().map_err(invalid)?;
    let store = Store::open(&state.database).map_err(internal)?;
    let mut conn = store.borrow_connection().map_err(internal)?;
    let tx = sjel_store::write_transaction(&mut conn).map_err(internal)?;
    let existing = tx.query_row(&format!("SELECT {COLUMNS} FROM inventory_mail_proposal WHERE source_id=?1 AND line_id=?2 AND event=?3"), params![body.source_id, body.line_id, body.event.as_str()], read_row).optional().map_err(internal)?;
    let proposal = if let Some(existing) = existing {
        if existing.stage != body {
            return Err(conflict("proposal identity already has different evidence"));
        }
        existing
    } else {
        tx.execute("INSERT INTO inventory_mail_proposal (source_id,line_id,event,title,evidence,data_class,order_ref,claim,extraction_version) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![body.source_id, body.line_id, body.event.as_str(), body.title, body.evidence, body.data_class, body.order_ref, body.claim, body.extraction_version]).map_err(internal)?;
        tx.query_row(
            &format!("SELECT {COLUMNS} FROM inventory_mail_proposal WHERE id=?1"),
            [tx.last_insert_rowid()],
            read_row,
        )
        .map_err(internal)?
    };
    tx.commit().map_err(internal)?;
    Ok(Json(serde_json::to_value(proposal).map_err(internal)?))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Withdraw {
    pub(crate) source_prefix: String,
}

/// One mail thread's prefix, `mail:v1:<thread>:`, so a withdrawal never spans threads.
pub(crate) fn thread_prefix(prefix: &str) -> bool {
    prefix
        .strip_prefix("mail:v1:")
        .and_then(|rest| rest.strip_suffix(':'))
        .is_some_and(|thread| {
            !thread.is_empty()
                && thread.len() <= 120
                && thread
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        })
}

/// Delete every undecided or rejected proposal from one thread, for example after the thread
/// became c3. Approved and matched proposals stay: a reviewer already accepted their evidence.
pub async fn withdraw(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Withdraw>,
) -> Result<Json<serde_json::Value>, Error> {
    if !thread_prefix(&body.source_prefix) {
        return Err(invalid("source_prefix must be mail:v1:<thread>:"));
    }
    let store = Store::open(&state.database).map_err(internal)?;
    let conn = store.borrow_connection().map_err(internal)?;
    // substr, not LIKE: `_` is a LIKE wildcard and is valid in a thread ID.
    let withdrawn = conn
        .execute(
            "DELETE FROM inventory_mail_proposal WHERE substr(source_id, 1, length(?1)) = ?1 AND (decision IS NULL OR decision = 'reject')",
            [&body.source_prefix],
        )
        .map_err(internal)?;
    Ok(Json(serde_json::json!({ "withdrawn": withdrawn })))
}

pub async fn list(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, Error> {
    let store = Store::open(&state.database).map_err(internal)?;
    let conn = store.borrow_connection().map_err(internal)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM inventory_mail_proposal ORDER BY id DESC"
        ))
        .map_err(internal)?;
    let proposals: Vec<Proposal> = stmt
        .query_map([], read_row)
        .map_err(internal)?
        .collect::<Result<_, _>>()
        .map_err(internal)?;
    Ok(Json(serde_json::json!({"proposals": proposals})))
}

pub async fn snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Error> {
    let store = Store::open(&state.database).map_err(internal)?;
    let conn = store.borrow_connection().map_err(internal)?;
    let snapshot = item_snapshot(&conn, &id)
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "item not found".into()))?;
    Ok(Json(serde_json::to_value(snapshot).map_err(internal)?))
}

pub(crate) fn validate_decision(body: &Decision) -> Result<(), Error> {
    if !matches!(body.action.as_str(), "approve" | "reject" | "match") || body.version < 1 {
        return Err(invalid("invalid action or version"));
    }
    if body.action != "match"
        && (body.expected_item_revision.is_some() || body.expected_state_id.is_some())
    {
        return Err(invalid("snapshot fields are only valid for match"));
    }
    if body.action == "reject" && (body.title.is_some() || body.item_id.is_some()) {
        return Err(invalid("reject takes no title or item_id"));
    }
    if body.action == "match"
        && (body.title.is_some()
            || !body.item_id.as_deref().is_some_and(|s| bounded(s, 256))
            || !body.expected_item_revision.is_some_and(|v| v > 0)
            || !body.expected_state_id.is_some_and(|v| v > 0))
    {
        return Err(invalid(
            "match requires item_id, expected_item_revision and expected_state_id; no title",
        ));
    }
    if body.action == "approve"
        && (!body.title.as_deref().is_some_and(|s| bounded(s, 256))
            || !body.item_id.as_deref().is_some_and(|s| bounded(s, 256)))
    {
        return Err(invalid("approve requires explicit title and item_id"));
    }
    Ok(())
}

pub(crate) fn validate_approval(stage: &Stage) -> Result<(), Error> {
    if stage.data_class == "c2" {
        return Err(conflict(
            "cannot create an inventory item from a c2 proposal: items do not preserve data class",
        ));
    }
    if stage.event != PurchaseEvent::Delivered {
        return Err(conflict("only delivery can create an item"));
    }
    Ok(())
}

pub async fn decide(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<Decision>,
) -> Result<Json<serde_json::Value>, Error> {
    validate_decision(&body)?;
    let store = Store::open(&state.database).map_err(internal)?;
    let mut conn = store.borrow_connection().map_err(internal)?;
    let tx = sjel_store::write_transaction(&mut conn).map_err(internal)?;
    let p = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM inventory_mail_proposal WHERE id=?1"),
            [id],
            read_row,
        )
        .optional()
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "proposal not found".into()))?;
    if p.decision.as_deref() == Some(body.action.as_str())
        && p.version.checked_sub(1) == Some(body.version)
        && p.item_id == body.item_id
        && p.approved_title == body.title
        && p.matched_item_revision == body.expected_item_revision
        && p.matched_state_id == body.expected_state_id
    {
        tx.commit().map_err(internal)?;
        return Ok(Json(serde_json::to_value(p).map_err(internal)?));
    }
    if p.version != body.version || p.decision.is_some() {
        return Err(conflict("proposal is stale or already decided"));
    }
    if body.action == "match" {
        let snapshot = item_snapshot(&tx, body.item_id.as_deref().unwrap_or_default())
            .map_err(internal)?
            .ok_or((StatusCode::NOT_FOUND, "item not found".into()))?;
        if snapshot.state.as_deref() != Some("owned")
            || Some(snapshot.item_revision) != body.expected_item_revision
            || snapshot.state_id != body.expected_state_id
        {
            return Err(conflict("item or state changed, or item is not owned"));
        }
    }
    if body.action == "approve" {
        validate_approval(&p.stage)?;
        let title = body.title.as_deref().unwrap_or_default();
        let item_id = body.item_id.as_deref().unwrap_or_default();
        let duplicate: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM inventory_item WHERE id=?1 OR lower(trim(label))=lower(trim(?2))) OR EXISTS(SELECT 1 FROM inventory_mail_proposal WHERE decision IN ('approve','match') AND order_ref IS NOT NULL AND order_ref=?3 AND line_id=?4)", params![item_id, title, p.stage.order_ref, p.stage.line_id], |r| r.get(0)).map_err(internal)?;
        if duplicate {
            return Err(conflict(
                "possible existing item or approved order line; review and match instead",
            ));
        }
        tx.execute("INSERT INTO inventory_item (id,kind,label,created_at,updated_at) VALUES (?1,'piece',?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![item_id, title]).map_err(internal)?;
        tx.execute("INSERT INTO inventory_item_state (item_id,state,since,note) VALUES (?1,'owned',strftime('%Y-%m-%dT%H:%M:%fZ','now'),'approved mail proposal')", [item_id]).map_err(internal)?;
    }
    let changed = tx.execute("UPDATE inventory_mail_proposal SET decision=?1,item_id=?2,approved_title=?3,matched_item_revision=?4,matched_state_id=?5,version=version+1 WHERE id=?6 AND version=?7 AND decision IS NULL", params![body.action, body.item_id, body.title, body.expected_item_revision, body.expected_state_id, id, body.version]).map_err(internal)?;
    if changed != 1 {
        return Err(conflict("proposal changed"));
    }
    let result = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM inventory_mail_proposal WHERE id=?1"),
            [id],
            read_row,
        )
        .map_err(internal)?;
    tx.commit().map_err(internal)?;
    Ok(Json(serde_json::to_value(result).map_err(internal)?))
}
