use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use turso::transaction::TransactionBehavior;
use turso::{params, Connection, Row};

use super::super::{boom, refused, HttpError};
use super::{aliases::item_snapshot, items, WalPreview};
use crate::mail_proposals::{
    thread_prefix, validate_approval, validate_decision, Decision, Proposal, Stage, Withdraw,
    COLUMNS,
};

type PreviewRouter = Router<Arc<WalPreview>>;

pub(super) fn reads(router: PreviewRouter) -> PreviewRouter {
    router
        .route("/api/mail-proposals", get(list))
        .route("/api/items/{id}/mail-match-snapshot", get(snapshot))
}

pub(super) fn writes(router: PreviewRouter) -> PreviewRouter {
    router
        .route(
            "/api/mail-proposals",
            post(stage).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/api/mail-proposals/withdraw",
            post(withdraw).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/api/mail-proposals/{id}/decision",
            post(decide).layer(DefaultBodyLimit::max(4096)),
        )
}

fn conflict(message: &str) -> HttpError {
    (StatusCode::CONFLICT, message.into())
}

fn proposal(row: &Row) -> Result<Proposal, HttpError> {
    let event = serde_json::from_value(Value::String(row.get(3).map_err(boom)?)).map_err(boom)?;
    Ok(Proposal {
        id: row.get(0).map_err(boom)?,
        stage: Stage {
            source_id: row.get(1).map_err(boom)?,
            line_id: row.get(2).map_err(boom)?,
            event,
            title: row.get(4).map_err(boom)?,
            evidence: row.get(5).map_err(boom)?,
            data_class: row.get(6).map_err(boom)?,
            order_ref: row.get(7).map_err(boom)?,
            claim: row.get(14).map_err(boom)?,
            extraction_version: row.get(15).map_err(boom)?,
        },
        version: row.get(8).map_err(boom)?,
        decision: row.get(9).map_err(boom)?,
        item_id: row.get(10).map_err(boom)?,
        approved_title: row.get(11).map_err(boom)?,
        matched_item_revision: row.get(12).map_err(boom)?,
        matched_state_id: row.get(13).map_err(boom)?,
    })
}

async fn by_id(conn: &Connection, id: i64) -> Result<Proposal, HttpError> {
    let mut rows = conn
        .query(
            format!("SELECT {COLUMNS} FROM inventory_mail_proposal WHERE id=?1"),
            [id],
        )
        .await
        .map_err(boom)?;
    rows.next()
        .await
        .map_err(boom)?
        .as_ref()
        .map(proposal)
        .transpose()?
        .ok_or((StatusCode::NOT_FOUND, "proposal not found".into()))
}

async fn stage_on(conn: &Connection, body: Stage) -> Result<Proposal, HttpError> {
    let mut rows=conn.query(format!("SELECT {COLUMNS} FROM inventory_mail_proposal WHERE source_id=?1 AND line_id=?2 AND event=?3"),
        params![body.source_id.as_str(),body.line_id.as_str(),body.event.as_str()]).await.map_err(boom)?;
    if let Some(row) = rows.next().await.map_err(boom)? {
        let existing = proposal(&row)?;
        if existing.stage != body {
            return Err(conflict("proposal identity already has different evidence"));
        }
        return Ok(existing);
    }
    drop(rows);
    conn.execute("INSERT INTO inventory_mail_proposal (source_id,line_id,event,title,evidence,data_class,order_ref,claim,extraction_version) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![body.source_id,body.line_id,body.event.as_str(),body.title,body.evidence,body.data_class,body.order_ref,body.claim,body.extraction_version]).await.map_err(boom)?;
    by_id(conn, conn.last_insert_rowid()).await
}

async fn stage(
    State(store): State<Arc<WalPreview>>,
    Json(body): Json<Stage>,
) -> Result<Json<Proposal>, HttpError> {
    body.validate().map_err(refused)?;
    let mut conn = store.writer().await?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let outcome = stage_on(&tx, body).await;
    Ok(Json(items::finish_write(tx, outcome).await?))
}

async fn withdraw(
    State(store): State<Arc<WalPreview>>,
    Json(body): Json<Withdraw>,
) -> Result<Json<Value>, HttpError> {
    if !thread_prefix(&body.source_prefix) {
        return Err(refused("source_prefix must be mail:v1:<thread>:"));
    }
    let conn = store.writer().await?;
    let withdrawn=conn.execute("DELETE FROM inventory_mail_proposal WHERE substr(source_id,1,length(?1))=?1 AND (decision IS NULL OR decision='reject')",[body.source_prefix]).await.map_err(boom)?;
    Ok(Json(json!({"withdrawn":withdrawn})))
}

async fn list(State(store): State<Arc<WalPreview>>) -> Result<Json<Value>, HttpError> {
    let conn = store.database.connect().map_err(boom)?;
    let mut rows = conn
        .query(
            format!("SELECT {COLUMNS} FROM inventory_mail_proposal ORDER BY id DESC"),
            (),
        )
        .await
        .map_err(boom)?;
    let mut proposals = Vec::new();
    while let Some(row) = rows.next().await.map_err(boom)? {
        proposals.push(proposal(&row)?);
    }
    Ok(Json(json!({"proposals":proposals})))
}

async fn snapshot(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<String>,
) -> Result<Json<inventory_api::MailMatchSnapshot>, HttpError> {
    let conn = store.database.connect().map_err(boom)?;
    let snapshot = item_snapshot(&conn, &id)
        .await?
        .ok_or((StatusCode::NOT_FOUND, "item not found".into()))?;
    Ok(Json(snapshot))
}

async fn approve(conn: &Connection, p: &Proposal, body: &Decision) -> Result<(), HttpError> {
    validate_approval(&p.stage)?;
    let title = body.title.as_deref().unwrap_or_default();
    let item_id = body.item_id.as_deref().unwrap_or_default();
    let mut rows=conn.query(
        "SELECT EXISTS(SELECT 1 FROM inventory_item WHERE id=?1 OR lower(trim(label))=lower(trim(?2)))
          OR EXISTS(SELECT 1 FROM inventory_mail_proposal WHERE decision IN ('approve','match') AND order_ref IS NOT NULL AND order_ref=?3 AND line_id=?4)",
        params![item_id,title,p.stage.order_ref.as_deref(),p.stage.line_id.as_str()],
    ).await.map_err(boom)?;
    let duplicate = rows
        .next()
        .await
        .map_err(boom)?
        .ok_or_else(|| boom("missing duplicate check"))?
        .get::<i64>(0)
        .map_err(boom)?
        != 0;
    if duplicate {
        return Err(conflict(
            "possible existing item or approved order line; review and match instead",
        ));
    }
    drop(rows);
    conn.execute("INSERT INTO inventory_item (id,kind,label,created_at,updated_at) VALUES (?1,'piece',?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![item_id,title]).await.map_err(boom)?;
    let added=conn.execute("INSERT INTO inventory_item_state (item_id,state,since,note) VALUES (?1,'owned',strftime('%Y-%m-%dT%H:%M:%fZ','now'),'approved mail proposal')",[item_id]).await.map_err(boom)?;
    if added != 1 {
        return Err(boom("approved item state was not recorded"));
    }
    Ok(())
}

async fn decide_on(conn: &Connection, id: i64, body: Decision) -> Result<Proposal, HttpError> {
    let p = by_id(conn, id).await?;
    if p.decision.as_deref() == Some(body.action.as_str())
        && p.version.checked_sub(1) == Some(body.version)
        && p.item_id == body.item_id
        && p.approved_title == body.title
        && p.matched_item_revision == body.expected_item_revision
        && p.matched_state_id == body.expected_state_id
    {
        return Ok(p);
    }
    if p.version != body.version || p.decision.is_some() {
        return Err(conflict("proposal is stale or already decided"));
    }
    match body.action.as_str() {
        "match" => {
            let snapshot = item_snapshot(conn, body.item_id.as_deref().unwrap_or_default())
                .await?
                .ok_or((StatusCode::NOT_FOUND, "item not found".into()))?;
            if snapshot.state.as_deref() != Some("owned")
                || Some(snapshot.item_revision) != body.expected_item_revision
                || snapshot.state_id != body.expected_state_id
            {
                return Err(conflict("item or state changed, or item is not owned"));
            }
        }
        "approve" => approve(conn, &p, &body).await?,
        _ => {}
    }
    let changed=conn.execute("UPDATE inventory_mail_proposal SET decision=?1,item_id=?2,approved_title=?3,matched_item_revision=?4,matched_state_id=?5,version=version+1 WHERE id=?6 AND version=?7 AND decision IS NULL",
        params![body.action,body.item_id,body.title,body.expected_item_revision,body.expected_state_id,id,body.version]).await.map_err(boom)?;
    if changed != 1 {
        return Err(conflict("proposal changed"));
    }
    by_id(conn, id).await
}

async fn decide(
    State(store): State<Arc<WalPreview>>,
    Path(id): Path<i64>,
    Json(body): Json<Decision>,
) -> Result<Json<Proposal>, HttpError> {
    validate_decision(&body)?;
    let mut conn = store.writer().await?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
        .map_err(boom)?;
    let result = decide_on(&tx, id, body).await;
    Ok(Json(items::finish_write(tx, result).await?))
}
