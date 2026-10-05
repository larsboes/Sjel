//! HTTP-Oberflaeche der Capability.
//!
//! Die Rechnung passiert hier, die Anzeige nicht: jeder Endpunkt liefert JSON, und das
//! Dashboard rendert es. Ein Frontend, das eigene Regeln auslegt, waere die Drift, die dieses
//! Programm verhindern soll.
//!
//! Gebunden wird ueber `sjel_server::serve_local` — Loopback, wie es die Bind-Policy von
//! `tools/doctor` fuer jede Capability in beiden Roots verlangt.
//!
//! ## Was hier NICHT steht, und warum
//!
//! **`/api/media`** (die Bilder unter `bild`) bleibt vorerst in `interior`. Es gehoert
//! hierher — ein Bild ist ein Feld einer Zeile —, aber der Medienpfad loest eine private
//! Asset-Wurzel auf, die `interior` auch fuer die RoomPlan-Aufnahmen benutzt. Ihn mitzunehmen,
//! ohne ihn zu verifizieren, waere ein Umzug mit einem stillen Verlust darin (ISA F13,
//! Schritt 3).
//!
//! **`/api/sync` gehoert hierher und liegt hier.** Der Geraete-Eingang traegt
//! Item-Mutationen; bis 2026-10-05 sass er in `interior`, womit eine zweite Capability in
//! dieselbe Tabelle schrieb (ISC-65).

use axum::body::{to_bytes, Body};
use axum::extract::{Extension, Path, Request, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
#[path = "sync.rs"]
mod sync;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

pub struct AppState {
    /// Das Geraeteregister, aus dem dieselbe Datei gelesen wird wie die Zeilen. Ein Feld und
    /// kein Aufruf je Anfrage, wie [`AppState::database`].
    pub device_store: devices::store::DevicesStore,
    /// Die SQLite-Datei, einmal beim Start aufgeloest. Ein Feld und kein Aufruf je Anfrage,
    /// damit ein Test den verdrahteten Router gegen eine Temp-Datei fahren kann statt gegen
    /// die Datenbank der Installation.
    pub database: PathBuf,
}

fn store(s: &AppState) -> Result<crate::store::Store, (axum::http::StatusCode, String)> {
    crate::store::Store::open(&s.database)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn boom<E: std::fmt::Display>(e: E) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn health() -> &'static str {
    "ok"
}

/// Was diese Capability beantwortet, als Daten neben `/health`. Ein veralteter Katalog waere
/// schlimmer als keiner, weil er geglaubt wird; `route_manifest::undeclared_routes` liest
/// deshalb den Router aus diesem Quelltext und faellt um, wenn eine Route hier fehlt.
const ROUTES: &[route_manifest::Route] = &[
    r(
        "GET",
        "/",
        "Alle Eintraege als Seite, bei jeder Anfrage aus den Zeilen erzeugt.",
    ),
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "Dieser Katalog."),
    r("GET", "/api/inventory", "Jeder Eintrag mit seinem Zustand."),
    r(
        "GET",
        "/api/wishlist",
        "Der offene Bedarf und was er in Monatssalden kostet.",
    ),
    r(
        "POST",
        "/api/items",
        "Ein Stueck oder einen Bedarf anlegen. `state` ist Pflicht.",
    ),
    r(
        "PUT",
        "/api/items/{id}",
        "Ein Stueck ersetzen. Nimmt die Item-Form, die /api/inventory liefert.",
    ),
    r(
        "PATCH",
        "/api/items/{id}",
        "Genannte Felder aendern, ungenannte in Ruhe lassen.",
    ),
    r(
        "GET",
        "/api/items/{id}/state",
        "Die Zustandsgeschichte eines Eintrags.",
    ),
    r(
        "POST",
        "/api/items/{id}/state",
        "Einen Zustandswechsel anhaengen.",
    ),
    r(
        "POST",
        "/api/vault/writeback",
        "Slots in die markierte Region des Vaults schreiben.",
    ),
    r(
        "POST",
        "/api/sync",
        "Signierte axon-sync/v1-Mutationen eines gepaarten Geraets.",
    ),
];

const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<serde_json::Value> {
    Json(route_manifest::manifest("inventory", ROUTES))
}

/// Die Seite ohne Shell — ein Blick direkt auf den Port, und der Weg fuer einen Menschen ohne
/// Dashboard. Sie rendert und rechnet nicht: was hier steht, steht in den Zeilen.
async fn index(
    State(s): State<Arc<AppState>>,
) -> Result<Html<String>, (axum::http::StatusCode, String)> {
    let rows = store(&s)?.catalogue().map_err(boom)?;
    let mut html = String::from(
        "<!doctype html><meta charset=\"utf-8\"><title>inventory</title>\
         <style>body{font:14px/1.5 -apple-system,sans-serif;margin:2rem;max-width:60rem}\
         td{padding:.2rem .6rem .2rem 0;vertical-align:top}code{color:#555}</style>\
         <h1>inventory</h1><p>Was ich besitze. Die Zeilen, keine Anzeige.</p><table>",
    );
    for (item, state) in rows.values() {
        html.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
            escape(&item.id),
            escape(&item.label),
            state.map(|s| s.as_str()).unwrap_or("—"),
        ));
    }
    html.push_str("</table>");
    Ok(Html(html))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn api_inventory(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let rows = store(&s)?.catalogue().map_err(boom)?;
    let out: Vec<_> = rows
        .into_values()
        .map(
            |(item, state)| serde_json::json!({ "item": item, "state": state.map(|s| s.as_str()) }),
        )
        .collect();
    Ok(Json(out))
}

/// Der offene Bedarf, und was er in Monatssalden kostet — die Naht zu `finance` (PRD B29).
/// Beide Zahlen kommen aus derselben Datei und keine ueber HTTP.
///
/// Zwei Summen statt einer, weil die Daten zwei Arten von Preis kennen: ein Produkt hat einen,
/// ein Slot hat eine Schaetzspanne. Sie in eine Zahl zu falten hiesse, eine Spanne als Preis
/// auszugeben, und das ist die Praezision, die sie nicht hat.
async fn api_wishlist(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let st = store(&s)?;
    let rows = st.catalogue().map_err(boom)?;
    let offen: Vec<_> = rows
        .values()
        .filter(|(_, s)| *s == Some(crate::store::State::Wanted))
        .map(|(i, _)| i)
        .collect();

    let untere: i64 = offen
        .iter()
        .map(|i| i.preis_cent.or(i.kosten_min_cent).unwrap_or(0))
        .sum();
    let obere: i64 = offen
        .iter()
        .map(|i| i.preis_cent.or(i.kosten_max_cent).unwrap_or(0))
        .sum();
    let ohne_preis = offen
        .iter()
        .filter(|i| i.preis_cent.is_none() && i.kosten_min_cent.is_none())
        .count();

    let conn = st.borrow_connection().map_err(boom)?;
    let saldo = crate::budget::monatssaldo(&conn).map_err(boom)?;
    let monate = saldo
        .as_ref()
        .and_then(|s| crate::budget::monate_bis_bezahlt(untere, s));

    Ok(Json(serde_json::json!({
        "items": offen,
        "summe_untere_kante_cent": untere,
        "summe_obere_kante_cent": obere,
        // Ein Posten ohne Preis zaehlt mit 0 in die Summe. Die Summe waere sonst still zu
        // klein, und diese Zahl ist die Warnung davor.
        "posten_ohne_preis": ohne_preis,
        "monatssaldo": saldo,
        "monate_bis_bezahlt": monate,
    })))
}

async fn api_vault_writeback(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let rows = store(&s)?.catalogue().map_err(boom)?;
    let Some(ergebnis) = crate::obsidian::writeback(&rows) else {
        return Err((
            axum::http::StatusCode::NOT_IMPLEMENTED,
            "keine Vault-Wurzel erklaert: obsidian.root in <overlay>/config/inventory.json setzen"
                .to_string(),
        ));
    };
    let report =
        ergebnis.map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "ok": report.conflicts.is_empty(),
        "seeded": report.seeded,
        "written": report.written,
        "unchanged": report.unchanged,
        "conflicts": report.conflicts,
    })))
}

#[derive(Deserialize)]
struct NewItem {
    #[serde(flatten)]
    item: crate::store::Item,
    /// Pflicht, und deshalb kein `Option`: ein Eintrag ohne Zustand taucht in keiner Liste auf,
    /// weil jede Abfrage auf den letzten Zustand joint. Ihn beim Anlegen zu vergessen hiesse,
    /// eine Zeile zu schreiben, die niemand je sieht.
    state: crate::store::State,
    #[serde(default)]
    note: Option<String>,
}

async fn api_post_item(
    State(s): State<Arc<AppState>>,
    Json(body): Json<NewItem>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let st = store(&s)?;
    if body.item.id.trim().is_empty() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "`id` fehlt".into()));
    }
    if st.item(&body.item.id).map_err(boom)?.is_some() {
        return Err((
            axum::http::StatusCode::CONFLICT,
            format!("`{}` gibt es schon — PATCH aendert ihn", body.item.id),
        ));
    }
    let revision = st.upsert_item(&body.item).map_err(boom)?;
    st.record_state(
        &body.item.id,
        body.state,
        body.note.as_deref().or(Some("in der Oberflaeche angelegt")),
    )
    .map_err(boom)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({
            "id": body.item.id, "state": body.state.as_str(), "ok": true, "revision": revision
        })),
    ))
}

/// Die erwartete Revision aus `If-Match` oder aus dem Rumpf (PRD §10 A5).
///
/// Nennen beide eine Zahl und nicht dieselbe, ist das ein Fehler und keine Vorauswahl.
fn erwartete_revision(
    headers: &axum::http::HeaderMap,
    body: &mut serde_json::Value,
) -> Result<Option<i64>, (axum::http::StatusCode, String)> {
    let aus_header = headers
        .get(axum::http::header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.parse::<i64>().map_err(|_| {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    format!("If-Match ist keine Zahl: `{v}`"),
                )
            })
        })
        .transpose()?;
    let aus_rumpf = body
        .get("expected_revision")
        .and_then(|v| v.as_i64())
        .map(Some)
        .unwrap_or(None);
    if let Some(im_rumpf) = body.as_object_mut() {
        im_rumpf.remove("expected_revision");
    }
    match (aus_header, aus_rumpf) {
        (Some(a), Some(b)) if a != b => Err((
            axum::http::StatusCode::BAD_REQUEST,
            format!("If-Match sagt {a}, expected_revision sagt {b}"),
        )),
        (Some(a), _) => Ok(Some(a)),
        (None, b) => Ok(b),
    }
}

async fn api_put_item(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(mut body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let erwartet = erwartete_revision(&headers, &mut body)?;
    let st = store(&s)?;
    let mut item: crate::store::Item = serde_json::from_value(body).map_err(|e| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            format!("Rumpf passt nicht auf einen Eintrag: {e}"),
        )
    })?;
    item.id = id;
    match erwartet {
        Some(e) => bedingt_schreiben(&st, &item, e),
        None => {
            let revision = st.upsert_item(&item).map_err(boom)?;
            Ok(geschrieben(&item.id, revision))
        }
    }
}

/// Genannte Felder aendern, ungenannte in Ruhe lassen.
///
/// `PUT` daneben ersetzt den ganzen Eintrag und ist damit fuer ein Formular die falsche Form:
/// `Item` fuehrt 53 Felder, eine Maske zeigt sechs, und was sie nicht schickt, waere weg. Das
/// ist derselbe stille Verlust, den `deny_unknown_fields` beim Import verhindert — nur in die
/// andere Richtung.
///
/// Zusammengefuehrt wird auf JSON-Ebene und nicht ueber eine zweite Struktur mit lauter
/// `Option<Option<_>>`: die Zeile ist die Wahrheit, also wird sie gelesen, mit dem Rumpf
/// ueberschrieben und zurueckgeschrieben. Ein ausdrueckliches `null` loescht ein Feld, ein
/// fehlender Schluessel laesst es stehen — der Unterschied, den ein Formular braucht.
async fn api_patch_item(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(mut patch): Json<serde_json::Value>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let erwartet = erwartete_revision(&headers, &mut patch)?;
    let st = store(&s)?;
    let (item, _) = st.item(&id).map_err(boom)?.ok_or((
        axum::http::StatusCode::NOT_FOUND,
        format!("kein Eintrag `{id}`"),
    ))?;
    let mut item = merge_patch(&item, patch)?;
    item.id = id;
    match erwartet {
        Some(e) => bedingt_schreiben(&st, &item, e),
        None => {
            let revision = st.upsert_item(&item).map_err(boom)?;
            Ok(geschrieben(&item.id, revision))
        }
    }
}

fn merge_patch(
    alt: &crate::store::Item,
    patch: serde_json::Value,
) -> Result<crate::store::Item, (axum::http::StatusCode, String)> {
    let serde_json::Value::Object(patch) = patch else {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "Rumpf ist kein Objekt".into(),
        ));
    };
    let mut merged = match serde_json::to_value(alt).map_err(|e| boom(e.to_string()))? {
        serde_json::Value::Object(m) => m,
        _ => unreachable!("Item serialisiert als Objekt"),
    };
    for (k, v) in patch {
        if k == "id" || k == "revision" {
            // Der Pfad gewinnt, wie bei PUT. Die Revision gehoert dem Server; wer sie als
            // Bedingung meint, schickt `If-Match`.
            continue;
        }
        if !merged.contains_key(&k) {
            return Err((
                axum::http::StatusCode::BAD_REQUEST,
                format!("`{k}` ist kein Feld eines Eintrags"),
            ));
        }
        merged.insert(k, v);
    }
    serde_json::from_value(serde_json::Value::Object(merged)).map_err(|e| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            format!("Rumpf passt nicht auf einen Eintrag: {e}"),
        )
    })
}

/// Die Antwort auf ein gelungenes Schreiben: die neue Revision im Rumpf **und** als `ETag`.
///
/// Das `ETag` ist keine Verzierung und stand bis 2026-10-05 nur in `capabilities/interior`,
/// obwohl die Zeilen schon hier lagen. Die Revision gehoert dem Server, und ein Client, der
/// sie als ETag weiterreicht, schickt sie beim naechsten Schreiben als `If-Match` zurueck —
/// genau der Kreis, den `api_patch_item` liest.
fn geschrieben(id: &str, revision: i64) -> Response {
    (
        [(axum::http::header::ETAG, format!("\"{revision}\""))],
        Json(serde_json::json!({ "id": id, "ok": true, "revision": revision })),
    )
        .into_response()
}

/// Schreiben gegen die gelesene Revision. Veraltet heisst 409 **mit** dem Stand, der jetzt
/// gilt: ein Client, der offline sein kann, muss ihn zeigen und den Menschen entscheiden
/// lassen, statt ihn mit einer neuen Revision automatisch zu wiederholen.
fn bedingt_schreiben(
    st: &crate::store::Store,
    item: &crate::store::Item,
    erwartet: i64,
) -> Result<Response, (axum::http::StatusCode, String)> {
    match st.update_item_if_revision(item, erwartet).map_err(boom)? {
        crate::store::Schreibergebnis::Geschrieben(revision) => Ok(geschrieben(&item.id, revision)),
        crate::store::Schreibergebnis::Fehlt => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("kein Eintrag `{}`", item.id),
        )),
        // Die Meldung nennt beide Revisionen und was zu tun ist, und das ist keine
        // Freundlichkeit: die Oberflaeche zeigt sie einem Menschen an, der entscheiden soll.
        // `dashboard/src/lib/api.ts` liest genau `error` aus diesem Rumpf, und der
        // Konflikttest in `dashboard/vite/interior-conflict.test.ts` haengt an dem Wort.
        crate::store::Schreibergebnis::Veraltet(aktuell, zustand) => Ok((
            axum::http::StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!(
                    "`{}` wurde inzwischen geaendert: erwartet Revision {erwartet}, aktuell {} — \
                     neu laden und die Aenderung auf den aktuellen Stand anwenden",
                    item.id, aktuell.revision
                ),
                "current": {
                    "item": aktuell,
                    "state": zustand.map(|s| s.as_str()),
                },
            })),
        )
            .into_response()),
    }
}

#[derive(Deserialize)]
struct StateBody {
    state: crate::store::State,
    #[serde(default)]
    note: Option<String>,
}

/// Einen Zustandswechsel anhaengen.
///
/// Anhaengen und nicht setzen: ein Wunsch, der gekauft wird, ist eine Zeile mehr und kein
/// ueberschriebenes Feld (PRD B25). Genau diese Spanne verbindet die Wunschliste mit `finance`,
/// und ein `UPDATE` haette sie gekostet.
async fn api_post_state(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<StateBody>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let st = store(&s)?;
    if st.item(&id).map_err(boom)?.is_none() {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("kein Eintrag `{id}`"),
        ));
    }
    let changed = st
        .record_state(&id, body.state, body.note.as_deref())
        .map_err(boom)?;
    Ok(Json(serde_json::json!({
        "id": id, "state": body.state.as_str(), "changed": changed
    })))
}

async fn api_state_history(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let rows = store(&s)?.state_history(&id).map_err(boom)?;
    let out: Vec<_> = rows
        .into_iter()
        .map(|(s, since, note)| serde_json::json!({ "state": s.as_str(), "since": since, "note": note }))
        .collect();
    Ok(Json(out))
}

// ─── Der Geraete-Eingang ──────────────────────────────────────────────────────

fn node_id() -> String {
    sjel_config::env_var("SJEL_NODE_ID").unwrap_or_else(|_| "node_mac".to_string())
}

/// Die Signatur des Geraets pruefen, bevor der Handler laeuft.
///
/// Der Rumpf wird dabei vollstaendig gelesen und weitergereicht: die Signatur deckt Methode,
/// Pfad **und** Rumpf, also kann sie nicht gegen einen Rumpf geprueft werden, den der Handler
/// spaeter liest. Ohne diese Middleware waere `/api/sync` ein unauthentifizierter
/// Schreibzugriff auf jede Zeile.
async fn signed_sync_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: middleware::Next,
) -> Response {
    let signed = match devices::auth::SignedRequest::from_headers(request.headers()) {
        Ok(signed) => signed,
        Err(error) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response()
        }
    };
    let method = request.method().as_str().to_string();
    let path_and_query = request.uri().path_and_query().map_or_else(
        || request.uri().path().to_string(),
        |value| value.as_str().to_string(),
    );
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, 8 * 1024 * 1024).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response(),
    };
    let result = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        let body = body.to_vec();
        move || {
            state
                .device_store
                .authenticate(&signed, &method, &path_and_query, &body)
        }
    })
    .await;
    let device = match result {
        Ok(Ok(device)) => device,
        Ok(Err(error)) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response()
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    };
    let mut request = Request::from_parts(parts, Body::from(body));
    request.extensions_mut().insert(device);
    next.run(request).await
}

/// Ein Buendel Mutationen von einem gepaarten Geraet annehmen.
///
/// **Die Entitaet heisst weiter `interior.item`.** Der Name stammt aus der Zeit, in der die
/// Zeilen `interior` gehoerten, und er ist Teil des Drahtformats: ein Telefon mit gefuellter
/// Outbox schickt ihn noch, und ein Umbenennen hier wuerde diese Mutationen abweisen, ohne
/// dass jemand es merkt — das stille Verlieren, das dieser Umzug vermeiden soll. Ein neuer
/// Name ist eine Protokollversion, kein Umbenennen.
///
/// Die Wiederholungssperre liegt in `{prefix}_sync_operation`: dieselbe `operation_id` ein
/// zweites Mal trifft einen Primaerschluessel und wird als `duplicate` beantwortet, statt die
/// Aenderung ein zweites Mal anzuwenden.
async fn api_sync(
    State(state): State<Arc<AppState>>,
    Extension(device): Extension<devices::store::Device>,
    Json(envelope): Json<sync::Envelope>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    if envelope.protocol_version != "axon-sync/v1" {
        return Err((
            StatusCode::BAD_REQUEST,
            "unsupported sync protocol version".into(),
        ));
    }
    if envelope.target_node_id != node_id() {
        return Err((
            StatusCode::BAD_REQUEST,
            "sync envelope targets a different node".into(),
        ));
    }
    if envelope.actor_device_id != device.id {
        return Err((
            StatusCode::UNAUTHORIZED,
            "envelope actor does not match the signed device".into(),
        ));
    }
    let st = store(&state)?;
    let mut response = sync::empty_response(&envelope, now);
    for mutation in envelope.mutations {
        let operation_id = mutation.operation_id.clone();
        let processed_at = now;
        if let Some(revision) = st.sync_operation_revision(&operation_id).map_err(boom)? {
            response.acknowledgements.push(sync::Acknowledgement {
                operation_id,
                status: "duplicate",
                revision: Some(revision.to_string()),
                processed_at,
                error: None,
            });
            continue;
        }
        if mutation.actor_device_id != device.id {
            response.acknowledgements.push(sync::Acknowledgement {
                operation_id,
                status: "rejected",
                revision: None,
                processed_at,
                error: Some("mutation actor does not match the signed device".into()),
            });
            continue;
        }
        if mutation.entity_type != "interior.item"
            || !matches!(mutation.action.as_str(), "patch" | "upsert")
        {
            response.acknowledgements.push(sync::Acknowledgement {
                operation_id,
                status: "rejected",
                revision: None,
                processed_at,
                error: Some(
                    "initial sync supports only interior.item patch or upsert mutations".into(),
                ),
            });
            continue;
        }
        let Some(base_revision) = mutation.base_revision.as_deref() else {
            response.acknowledgements.push(sync::Acknowledgement {
                operation_id,
                status: "rejected",
                revision: None,
                processed_at,
                error: Some("base_revision is required for an offline mutation".into()),
            });
            continue;
        };
        let expected = match base_revision.parse::<i64>() {
            Ok(value) => value,
            Err(_) => {
                response.acknowledgements.push(sync::Acknowledgement {
                    operation_id,
                    status: "rejected",
                    revision: None,
                    processed_at,
                    error: Some("base_revision must be a decimal revision".into()),
                });
                continue;
            }
        };
        let current = st.item(&mutation.entity_id).map_err(boom)?.ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("no item `{}`", mutation.entity_id),
            )
        })?;
        let mut merged = merge_patch(
            &current.0,
            serde_json::Value::Object(mutation.fields.clone()),
        )?;
        merged.id = mutation.entity_id.clone();
        match st
            .update_item_if_revision(&merged, expected)
            .map_err(boom)?
        {
            crate::store::Schreibergebnis::Geschrieben(revision) => {
                st.record_sync_operation(&mutation.operation_id, revision)
                    .map_err(boom)?;
                response.acknowledgements.push(sync::Acknowledgement {
                    operation_id,
                    status: "accepted",
                    revision: Some(revision.to_string()),
                    processed_at,
                    error: None,
                });
            }
            crate::store::Schreibergebnis::Fehlt => {
                response.acknowledgements.push(sync::Acknowledgement {
                    operation_id,
                    status: "rejected",
                    revision: None,
                    processed_at,
                    error: Some("item does not exist".into()),
                })
            }
            crate::store::Schreibergebnis::Veraltet(actual, state) => {
                let canonical = serde_json::to_value(&*actual).unwrap_or(serde_json::Value::Null);
                response.conflicts.push(serde_json::json!({
                    "conflict_id": mutation.operation_id,
                    "operation_id": mutation.operation_id,
                    "entity_type": mutation.entity_type,
                    "entity_id": mutation.entity_id,
                    "base_revision": mutation.base_revision,
                    "canonical_revision": actual.revision.to_string(),
                    "local_fields": mutation.fields,
                    "canonical_fields": canonical,
                    "detected_at": processed_at,
                    "state": "open"
                }));
                response.acknowledgements.push(sync::Acknowledgement {
                    operation_id,
                    status: "conflict",
                    revision: Some(actual.revision.to_string()),
                    processed_at,
                    error: Some(format!("canonical revision is {}", actual.revision)),
                });
                let _ = state;
            }
        }
    }
    Ok(Json(response))
}

/// Der Name dieser Capability, fuer die Umgebungsvariable der Origin-Sperre
/// (`SJEL_INVENTORY_ALLOWED_ORIGIN_HOSTS`).
const CAPABILITY: &str = "inventory";

pub fn build_router(state: Arc<AppState>) -> Router {
    let signed_sync =
        Router::new()
            .route("/api/sync", post(api_sync))
            .layer(middleware::from_fn_with_state(
                Arc::clone(&state),
                signed_sync_auth,
            ));
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/routes", get(routes))
        .route("/api/inventory", get(api_inventory))
        .route("/api/wishlist", get(api_wishlist))
        .route("/api/items", post(api_post_item))
        .route("/api/items/{id}", put(api_put_item).patch(api_patch_item))
        .route(
            "/api/items/{id}/state",
            get(api_state_history).post(api_post_state),
        )
        .route("/api/vault/writeback", post(api_vault_writeback))
        .merge(signed_sync)
        // Die Sperre liegt unter allen Routen, weil axum nur die Routen umhuellt, die VOR
        // einem `.layer()`-Aufruf registriert wurden (axum 0.7 `src/docs/routing/layer.md`).
        //
        // Sie ist mit `POST /api/vault/writeback` hierher gezogen und nicht freiwillig: diese
        // Route nimmt keinen Request-Body, ist damit eine *einfache* Anfrage im Sinne von
        // CORS und laeuft ohne Preflight. Eine fremde Seite konnte sie also ausloesen, und ob
        // der Browser die Antwort danach weiterreicht, ist fuer einen Schreibvorgang
        // gleichgueltig. `capabilities/interior` trug die Sperre, solange die Route dort lag.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

/// Baut den Zustand aus der Installation und bindet Loopback.
pub async fn serve() {
    let port: u16 = sjel_config::env_var("SJEL_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8101);
    let database = sjel_config::database_path();
    let device_store = devices::store::DevicesStore::open(&database)
        .unwrap_or_else(|error| panic!("inventory: cannot open device registry: {error}"));
    let state = Arc::new(AppState {
        device_store,
        database,
    });
    sjel_server::serve_local("inventory", port, build_router(state)).await;
}

#[cfg(test)]
mod route_manifest_tests {
    /// Ein Katalog, der luegt, wird geglaubt. Das hier liest den Router aus seinem eigenen
    /// Quelltext, also faellt ein `.route()` ohne Zusammenfassung hier um, statt eine
    /// Oberflaeche auszuliefern, die sich selbst falsch beschreibt.
    #[test]
    fn der_katalog_nennt_jede_ausgelieferte_route() {
        let fehlend = route_manifest::undeclared_routes(include_str!("api.rs"), super::ROUTES);
        assert!(
            fehlend.is_empty(),
            "diese Routen stehen nicht im Katalog: {fehlend:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Item;
    use serde_json::json;

    fn hemd() -> Item {
        Item {
            id: "hemd".into(),
            label: "Ein Hemd".into(),
            category: Some("kleidung".into()),
            preis_cent: Some(7990),
            ..Default::default()
        }
    }

    /// Der eigentliche Punkt von PATCH: ein Formular zeigt sechs Felder, der Eintrag hat 53,
    /// und die ungenannten 47 muessen den Vorgang ueberleben.
    #[test]
    fn ein_benanntes_feld_aendert_sich_und_die_uebrigen_bleiben() {
        let neu = merge_patch(&hemd(), json!({ "preis_cent": 8990 })).expect("Patch passt");
        assert_eq!(neu.preis_cent, Some(8990));
        assert_eq!(neu.label, "Ein Hemd");
        assert_eq!(neu.category.as_deref(), Some("kleidung"));
    }

    #[test]
    fn ein_unbekanntes_feld_wird_abgelehnt_und_nicht_ignoriert() {
        let e = merge_patch(&hemd(), json!({ "tiefe": 42 })).expect_err("`tiefe` gibt es nicht");
        assert!(e.1.contains("tiefe"));
    }

    /// `null` loescht, ein fehlender Schluessel laesst stehen — der Unterschied, den ein
    /// Formular braucht.
    #[test]
    fn null_loescht_und_ein_fehlender_schluessel_nicht() {
        let geloescht = merge_patch(&hemd(), json!({ "preis_cent": null })).expect("null ist ok");
        assert_eq!(geloescht.preis_cent, None);
        let unberuehrt = merge_patch(&hemd(), json!({ "label": "Anders" })).expect("ok");
        assert_eq!(unberuehrt.preis_cent, Some(7990));
    }
}

/// Der HTTP-Vertrag von PRD §10 A5 am verdrahteten Router, gegen eine Temp-Datei.
///
/// Die Rennbedingung selbst prueft `tests/revision.rs` mit echten Faeden; hier geht es darum,
/// was ein Client sieht: welcher Status, welcher Rumpf, welcher Header. Der Test ist am
/// 2026-10-05 mit der Item-Oberflaeche aus `capabilities/interior` hierher gezogen, samt dem
/// `ETag`, den diese Capability bis dahin nicht gesetzt hat.
#[cfg(test)]
mod revision_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, HeaderMap, Request};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    fn router(name: &str) -> (Router, PathBuf) {
        let pfad = std::env::temp_dir().join(format!(
            "inventory-api-revision-{name}-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&pfad);
        let device_store = devices::store::DevicesStore::open(&pfad).unwrap();
        let r = build_router(Arc::new(AppState {
            device_store,
            database: pfad.clone(),
        }));
        (r, pfad)
    }

    async fn senden(
        r: &Router,
        methode: &str,
        pfad: &str,
        if_match: Option<&str>,
        rumpf: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut anfrage = Request::builder()
            .method(methode)
            .uri(pfad)
            .header("content-type", "application/json");
        if let Some(v) = if_match {
            anfrage = anfrage.header("if-match", v);
        }
        let antwort = r
            .clone()
            .oneshot(anfrage.body(Body::from(rumpf.to_string())).unwrap())
            .await
            .expect("der Router antwortet");
        let status = antwort.status();
        let headers = antwort.headers().clone();
        let bytes = axum::body::to_bytes(antwort.into_body(), usize::MAX)
            .await
            .unwrap();
        let wert = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, headers, wert)
    }

    fn schrank(label: &str) -> Value {
        json!({ "id": "schrank", "kind": "piece", "label": label, "b": 100 })
    }

    #[tokio::test]
    async fn ohne_bedingung_bleibt_alles_wie_vorher_und_die_revision_steht_in_jeder_antwort() {
        let (r, pfad) = router("ohne");
        let (s, h, v) = senden(&r, "PUT", "/api/items/schrank", None, schrank("a")).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["revision"], 1);
        assert_eq!(h[header::ETAG], "\"1\"");

        let (s, _, v) = senden(&r, "PATCH", "/api/items/schrank", None, json!({"b": 120})).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["revision"], 2);

        let (s, _, v) = senden(&r, "GET", "/api/inventory", None, Value::Null).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v[0]["item"]["revision"], 2, "{v}");
        let _ = std::fs::remove_file(&pfad);
    }

    #[tokio::test]
    async fn eine_passende_revision_schreibt_und_erhoeht_um_eins() {
        let (r, pfad) = router("passt");
        senden(&r, "PUT", "/api/items/schrank", None, schrank("a")).await;

        // Wie ein ETag, in Anfuehrungszeichen.
        let (s, h, v) = senden(&r, "PUT", "/api/items/schrank", Some("\"1\""), schrank("b")).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["revision"], 2);
        assert_eq!(h[header::ETAG], "\"2\"");

        // Und ohne, von Hand.
        let (s, _, v) = senden(
            &r,
            "PATCH",
            "/api/items/schrank",
            Some("2"),
            json!({"b": 90}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["revision"], 3);

        // Das Rumpffeld ist der Ersatzweg und wird nicht als Eintragsfeld abgewiesen.
        let (s, _, v) = senden(
            &r,
            "PATCH",
            "/api/items/schrank",
            None,
            json!({"b": 80, "expected_revision": 3}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["revision"], 4);
        let _ = std::fs::remove_file(&pfad);
    }

    #[tokio::test]
    async fn eine_veraltete_revision_bekommt_409_mit_dem_aktuellen_stand() {
        let (r, pfad) = router("veraltet");
        senden(&r, "PUT", "/api/items/schrank", None, schrank("a")).await;
        senden(
            &r,
            "PUT",
            "/api/items/schrank",
            Some("1"),
            schrank("vom Mac"),
        )
        .await;

        for (methode, rumpf) in [
            ("PUT", schrank("vom Telefon")),
            ("PATCH", json!({"label": "vom Telefon"})),
        ] {
            let (s, _, v) = senden(&r, methode, "/api/items/schrank", Some("\"1\""), rumpf).await;
            assert_eq!(s, StatusCode::CONFLICT, "{methode}: {v}");
            assert_eq!(v["current"]["item"]["label"], "vom Mac");
            assert_eq!(v["current"]["item"]["revision"], 2);
            let fehler = v["error"].as_str().expect("error ist ein Text");
            assert!(fehler.contains("inzwischen geaendert"), "{fehler}");
        }
        // Dasselbe ueber das Rumpffeld.
        let (s, _, v) = senden(
            &r,
            "PATCH",
            "/api/items/schrank",
            None,
            json!({"label": "x", "expected_revision": 1}),
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT, "{v}");

        let (_, _, v) = senden(&r, "GET", "/api/inventory", None, Value::Null).await;
        assert_eq!(
            v[0]["item"]["label"], "vom Mac",
            "nichts wurde ueberschrieben"
        );
        let _ = std::fs::remove_file(&pfad);
    }

    #[tokio::test]
    async fn ein_fehlender_eintrag_bleibt_404() {
        let (r, pfad) = router("fehlt");
        let (s, _, _) = senden(&r, "PATCH", "/api/items/nichts", None, json!({"b": 1})).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _, _) = senden(&r, "PATCH", "/api/items/nichts", Some("1"), json!({"b": 1})).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        // PUT mit Bedingung legt nichts an: eine Revision fuer eine Zeile, die es nicht gibt.
        let (s, _, _) = senden(&r, "PUT", "/api/items/schrank", Some("1"), schrank("a")).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, _, v) = senden(&r, "GET", "/api/inventory", None, Value::Null).await;
        assert_eq!(v, json!([]));
        let _ = std::fs::remove_file(&pfad);
    }

    #[tokio::test]
    async fn eine_unlesbare_oder_widerspruechliche_bedingung_wird_abgewiesen() {
        let (r, pfad) = router("unlesbar");
        senden(&r, "PUT", "/api/items/schrank", None, schrank("a")).await;
        let (s, _, _) = senden(&r, "PUT", "/api/items/schrank", Some("*"), schrank("b")).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let mut rumpf = schrank("b");
        rumpf["expected_revision"] = json!(2);
        let (s, _, _) = senden(&r, "PUT", "/api/items/schrank", Some("1"), rumpf).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let _ = std::fs::remove_file(&pfad);
    }
}
