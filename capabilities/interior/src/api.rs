//! HTTP-Oberflaeche der Capability.
//!
//! Die Rechnung passiert hier, die Anzeige nicht: jeder Endpunkt liefert entweder JSON oder
//! fertiges SVG/HTML, das aus dem Modell erzeugt wurde. Ein Frontend, das eigene Masse haelt
//! oder eigene Regeln auslegt, waere die Drift, die dieses Programm verhindern soll.
//!
//! Gebunden wird ueber `sjel_server::serve_local` — Loopback, wie es die Bind-Policy von
//! `axon doctor` fuer jede Capability in beiden Roots verlangt.

use crate::clearance::check_layout;
use crate::model::Model;
use crate::plan;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

struct AppState {
    flat: String,
    /// Die SQLite-Datei, einmal beim Start aufgeloest. Ein Feld und kein Aufruf je Anfrage,
    /// damit ein Test den verdrahteten Router gegen eine Temp-Datei fahren kann statt gegen
    /// die Datenbank der Installation.
    database: PathBuf,
}

/// Das Modell wird pro Anfrage frisch gelesen, nicht beim Start zwischengespeichert.
///
/// Absicht: die TOML-Dateien sind von Hand editierbar und werden von Hand editiert. Ein
/// Prozess, der beim Start eine Kopie zieht, zeigt nach der ersten Korrektur einen Plan, der
/// den Zahlen widerspricht, aus denen er zu stammen behauptet. Das Lesen kostet unter einer
/// Millisekunde; ein veralteter Plan kostet eine Fehlentscheidung.
fn load(state: &AppState) -> Result<Model, (StatusCode, String)> {
    Model::load(&state.flat).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn health() -> &'static str {
    "ok"
}

/// Was diese Capability beantwortet, als Daten neben `/health` — dieselbe Zusage wie in jeder
/// anderen Capability mit HTTP-Oberflaeche. Ein veralteter Katalog waere schlimmer als keiner,
/// weil er geglaubt wird; `route_manifest::undeclared_routes` liest deshalb unten den Router
/// aus diesem Quelltext und faellt um, wenn eine Route hier fehlt.
const ROUTES: &[route_manifest::Route] = &[
    r(
        "GET",
        "/",
        "Alle Layouts als Side, bei jeder Anfrage aus dem Modell erzeugt.",
    ),
    r("GET", "/health", "Liveness."),
    r(
        "GET",
        "/api/media/{*pfad}",
        "Ein Bild aus dem privaten Asset-Verzeichnis. Nur von dort, und nur auf Anfrage.",
    ),
    r(
        "GET",
        "/api/roomplan/reference",
        "Die aktuellste native RoomPlan-Referenz als Metadaten, ohne absolute Pfade.",
    ),
    r(
        "GET",
        "/api/roomplan/asset",
        "Die aktuellste native RoomPlan-Referenz als unveraenderte USDZ-Datei.",
    ),
    r(
        "GET",
        "/api/roomplan/revisions",
        "Synchronisierte semantische RoomPlan-Revisionen fuer den Vergleich.",
    ),
    r(
        "POST",
        "/api/roomplan/revisions/{revision_id}/review",
        "Eine RoomPlan-Revision annehmen oder ablehnen, ohne room.toml zu aendern.",
    ),
    r(
        "GET",
        "/api/layouts/{name}/allowed",
        "Erlaubte Positionen eines Stuecks als Lauflaengen. Harte Kanten fuers Ziehen. ?ref=&rot=",
    ),
    r(
        "POST",
        "/api/layouts/{name}/preview",
        "Verdikt und Plan zu einer Aufstellung, ohne sie zu schreiben. Fuers Drehen noetig.",
    ),
    r(
        "POST",
        "/api/layouts",
        "Ein Layout anlegen: {id, name?, von?, items?, notiz?}. Ohne von und items ist es leer.",
    ),
    r(
        "DELETE",
        "/api/layouts/{name}",
        "Ein Layout aus der Liste nehmen. Es wandert nach layouts/archiv/ und wird nie geloescht.",
    ),
    r(
        "PUT",
        "/api/layouts/{name}",
        "Die Positionen eines Layouts ersetzen und sofort neu pruefen. Der Kopf der Datei bleibt.",
    ),
    r(
        "PUT",
        "/api/placements",
        "Wo die Stuecke wirklich stehen, fuer die aktive Wohnung. Body: {items}.",
    ),
    r(
        "POST",
        "/api/placements/preview",
        "Der Ist-Zustand als Plan und Verdikt, ohne Layoutdatei. Body: {items}.",
    ),
    r(
        "POST",
        "/api/placements/allowed",
        "Erlaubte Positionen eines Stuecks im Ist-Zustand. Body: {items, ref, rot}.",
    ),
    r(
        "POST",
        "/api/items/{id}/impact",
        "Was ein Feld mit den Verdikten machen wuerde, ohne es zu schreiben.",
    ),
    r("GET", "/routes", "Dieser Katalog."),
    r(
        "GET",
        "/api/layouts/{name}/toleranz",
        "Bis zu welchem Messfehler das Verdikt haelt, und woran es dann kippt.",
    ),
    r(
        "GET",
        "/api/layouts/{name}/sonne",
        "Wann im Jahr welches Stueck in direkter Sonne steht. Braucht [lage] in room.toml.",
    ),
    r(
        "GET",
        "/api/layouts/{name}/einbringung",
        "Kommt jedes Stueck durch die Tuer und bis an seinen Platz.",
    ),
    r(
        "GET",
        "/api/passt",
        "Passt ein gedachtes Stueck durch die Tuer? ?b=&t=&zerlegbar= — die Frage vor dem Kauf.",
    ),
    r(
        "GET",
        "/api/deklaration",
        "Wer wird noch am Namen gemessen, was waere die Zeile, und was aendert sie.",
    ),
    r(
        "GET",
        "/api/kaufen",
        "Welcher Bedarf zuerst, kumuliert, und wann er aus dem Monatssaldo erreicht ist.",
    ),
    r(
        "POST",
        "/api/search",
        "Eine Suche anstossen. Antwortet 202 mit einer Auftragsnummer, nicht mit dem Ergebnis.",
    ),
    r(
        "POST",
        "/api/compose",
        "Eine ganze Wohnung stellen lassen. Ebenfalls ein Auftrag: die Strahlsuche rechnet Minuten.",
    ),
    r(
        "GET",
        "/api/auftraege/{id}",
        "Was aus einem Auftrag geworden ist: laeuft, fertig mit Ergebnis, oder gescheitert.",
    ),
    r(
        "GET",
        "/api/model",
        "Der gemessene Raum: Masse, Polygon, Waende, Oeffnungen, und was daran geschaetzt ist.",
    ),
    r(
        "GET",
        "/api/layouts",
        "Jedes Layout mit Verdikt, Verstosszahlen und Korridorbreiten.",
    ),
    r(
        "GET",
        "/api/layouts/{name}",
        "Ein Layout: die volle Pruefung und der fertige Plan als SVG.",
    ),
    r(
        "GET",
        "/api/flats",
        "Welche Wohnungen es gibt und gegen welche dieser Prozess rechnet.",
    ),
    r(
        "GET",
        "/api/placements/{flat}",
        "Wo die Stuecke in dieser Wohnung tatsaechlich stehen.",
    ),
    r(
        "PUT",
        "/api/placements/{flat}/{item}",
        "Ein Stueck in dieser Wohnung platzieren. Body: {x, y, rot}.",
    ),
];

/// Kurzform, damit die Tabelle oben wie eine Tabelle liest.
const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> Json<serde_json::Value> {
    Json(route_manifest::manifest("interior", ROUTES))
}

async fn api_model(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    let (b, t) = m.room.dimensions();
    Ok(Json(serde_json::json!({
        "flat": m.room.flat,
        "area_m2": m.room.area_m2(),
        // Die aeusseren Masse, damit die Oberflaeche sie neben den Plan schreiben kann, ohne
        // sich das Polygon selbst auszumessen.
        "masse": { "b": b, "t": t },
        "hoehe": m.room.main_room.height,
        "polygon": m.room.main_room.polygon,
        "bad": m.room.bathroom,
        "terrasse": m.room.terrace,
        "waende": m.room.walls,
        "oeffnungen": m.room.openings,
        "fix_moebel": m.room.fixed_furniture,
        "todo": m.room.todo.open,
        "katalog_groesse": m.catalogue.len(),
        "ungemessen": m.uncertainties().into_iter()
            .map(|(id, label, f)| serde_json::json!({ "id": id, "label": label, "felder": f }))
            .collect::<Vec<_>>(),
    })))
}

async fn api_layouts(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    let names = m
        .layout_names()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut out = Vec::new();
    for n in names {
        let Ok(l) = m.load_layout(&n) else { continue };
        let Ok(r) = check_layout(&m, &l) else {
            continue;
        };
        out.push(serde_json::json!({
            "id": n, "name": l.name, "pass": r.pass,
            "hard": r.hard.len(), "soft": r.soft.len(),
            "corridors": r.metrics.corridors,
            "occupied_m2": r.metrics.occupied_area_m2,
        }));
    }
    Ok(Json(out))
}

async fn api_layout(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let m = load(&s)?;
    let l = m
        .load_layout(&name)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let r = check_layout(&m, &l).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let svg = plan::svg(&m, &l).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

fn store(s: &AppState) -> Result<crate::store::Store, (StatusCode, String)> {
    crate::store::Store::open(&s.database)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn boom<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// Welche Wohnungen unter `flats/` liegen, und welche dieser Prozess bedient.
///
/// Die uebrigen Endpunkte antworten fuer GENAU EINE Wohnung — die aus `SJEL_INTERIOR_FLAT`
/// oder die einzige vorhandene. Das ist heute richtig und wird es nicht bleiben: PRD B28 will
/// zwei Raeume nebeneinander, und dann traegt jeder Pfad die Wohnung. Bis dahin sagt dieser
/// Endpunkt wenigstens, dass es eine Auswahl gibt, statt sie zu verschweigen.
async fn api_flats(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let alle =
        crate::model::flats().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "flats": alle, "aktiv": s.flat })))
}

async fn api_placements(
    State(s): State<Arc<AppState>>,
    Path(flat): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    Ok(Json(store(&s)?.placements(&flat).map_err(boom)?))
}

/// Was eine Aenderung mit den Verdikten machen WUERDE, ohne sie zu schreiben.
///
/// Der Grund, aus dem das ein Endpunkt ist und kein Kommentar in einer Anleitung: die
/// Raeumungsfelder aus PRD Q61 sind genau die, deren Wirkung man nicht sieht, bevor man sie
/// setzt. `opens` am Kleiderschrank kostet je nach Richtung 2 oder 4 Layouts, und das stand
/// nirgends — es musste am 2026-08-31 von Hand ausgerechnet werden, einmal je Richtung. Wer ein
/// Feld in der Oberflaeche fuellt, soll dieselbe Rechnung sehen, bevor er speichert.
///
/// Schreibt nichts. Der Katalog wird im Speicher gepatcht und jedes Layout neu gerechnet.
async fn api_item_impact(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(patch): Json<serde_json::Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let before = verdicts(&model)?;

    let alt = model
        .catalogue
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("kein Eintrag `{id}`")))?;
    let neu = sjel_inventory::api::merge_patch(alt, patch)?;

    let mut model = load(&s)?;
    model.catalogue.insert(id.clone(), neu);
    let after = verdicts(&model)?;

    let mut changed = Vec::new();
    for (name, (pass, hard, soft)) in &before {
        let Some((p2, h2, s2)) = after.get(name) else {
            continue;
        };
        if pass != p2 || hard != h2 || soft != s2 {
            changed.push(serde_json::json!({
                "layout": name,
                "vorher": { "pass": pass, "hard": hard, "soft": soft },
                "nachher": { "pass": p2, "hard": h2, "soft": s2 },
            }));
        }
    }
    Ok(Json(serde_json::json!({
        "item": id,
        "layouts": before.len(),
        "bestanden_vorher": before.values().filter(|(p, _, _)| *p).count(),
        "bestanden_nachher": after.values().filter(|(p, _, _)| *p).count(),
        "geaendert": changed,
    })))
}

type Verdict = std::collections::BTreeMap<String, (bool, Vec<String>, Vec<String>)>;

fn verdicts(model: &Model) -> Result<Verdict, (StatusCode, String)> {
    let mut out = Verdict::new();
    for name in model.layout_names().map_err(|e| boom(e.to_string()))? {
        let Ok(l) = model.load_layout(&name) else {
            continue;
        };
        let Ok(r) = check_layout(model, &l) else {
            continue;
        };
        let mut hard: Vec<String> = r.hard.iter().map(|v| v.rule.clone()).collect();
        let mut soft: Vec<String> = r.soft.iter().map(|v| v.rule.clone()).collect();
        hard.sort();
        soft.sort();
        out.insert(name, (r.pass, hard, soft));
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
struct PlacementBody {
    x: i32,
    y: i32,
    #[serde(default)]
    rot: i32,
}

async fn api_put_placement(
    State(s): State<Arc<AppState>>,
    Path((flat, item)): Path<(String, String)>,
    Json(body): Json<PlacementBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    store(&s)?
        .place(&crate::store::Placement {
            item_id: item.clone(),
            flat: flat.clone(),
            x: body.x,
            y: body.y,
            rot: body.rot,
        })
        .map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "flat": flat, "item": item, "ok": true }),
    ))
}

#[derive(serde::Deserialize)]
struct LayoutBody {
    items: Vec<crate::model::PlacedItem>,
}

/// Die Positionen eines bestehenden Layouts ersetzen und sofort neu pruefen.
///
/// Antwortet mit dem Verdikt und dem fertigen Plan, damit die Oberflaeche nach einem Zug nichts
/// selbst zu rechnen hat. Der Kopf der Datei bleibt stehen — `layout_io` erhaelt ihn, und wo er
/// nicht genuegt, bricht es ab statt zu kuerzen.
async fn api_put_layout(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    crate::layout_io::update(&model, &id, &body.items)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let l = model
        .load_layout(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let r = check_layout(&model, &l).map_err(boom)?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

/// Ein Bild aus dem konfigurierten privaten Asset-Verzeichnis, auf Anfrage und nur von dort.
///
/// **Der Pfad kommt vom Client, also wird er aufgeloest und geprueft, nicht zusammengesetzt.**
/// `canonicalize` beidseitig und dann ein `starts_with`: ein `..`, ein absoluter Pfad oder ein
/// Symlink, der aus dem Verzeichnis zeigt, faellt damit auf, statt eine Datei auszuliefern, die
/// niemand gemeint hat. Eine Pruefung auf die Zeichenfolge `..` allein waere die Fassung, die
/// bei einem Symlink still versagt.
///
/// `service.toml` nennt diese Trennung als den Grund, aus dem die Capability oeffentlich stehen
/// darf: das Bundle enthaelt kein Foto, die privaten Assets liegen ausserhalb des Bundles, und
/// geliefert wird erst auf Anfrage.
///
/// Jeder Dateizugriff hier laeuft ueber `tokio::fs`, nicht ueber `std::fs`. Das ist der einzige
/// Handler im Repo, der ein ganzes Bild liest, und ein `std::fs::read` in einem `async fn` haelt
/// einen Worker-Thread der Laufzeit an, statt nur diese eine Anfrage warten zu lassen. Die
/// Pfadpruefung bleibt davon unberuehrt: `canonicalize` und `starts_with` sagen dasselbe.
async fn latest_roomplan_capture(
    flat: &str,
) -> Result<(PathBuf, PathBuf, PathBuf), (StatusCode, String)> {
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(flat);
    let mut entries = tokio::fs::read_dir(&root).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            "keine RoomPlan-Aufnahmen".to_string(),
        )
    })?;
    let mut dates = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(boom)? {
        if entry.file_type().await.map_err(boom)?.is_dir() {
            dates.push(entry.path());
        }
    }
    dates.sort();
    for directory in dates.into_iter().rev() {
        let asset = directory.join("captured-room.usdz");
        let observation = directory.join("observation.json");
        if tokio::fs::try_exists(&asset).await.map_err(boom)?
            && tokio::fs::try_exists(&observation).await.map_err(boom)?
        {
            return Ok((directory, asset, observation));
        }
    }
    Err((
        StatusCode::NOT_FOUND,
        "keine vollstaendige RoomPlan-Aufnahme".to_string(),
    ))
}

async fn api_roomplan_reference(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (directory, asset, observation) = latest_roomplan_capture(&s.flat).await?;
    let observation: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&observation).await.map_err(boom)?)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let manifest_path = directory.join("sync-manifest.json");
    let manifest = if tokio::fs::try_exists(&manifest_path).await.map_err(boom)? {
        Some(
            serde_json::from_slice::<serde_json::Value>(
                &tokio::fs::read(manifest_path).await.map_err(boom)?,
            )
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        )
    } else {
        None
    };
    let bytes = tokio::fs::read(&asset).await.map_err(boom)?;
    let sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(Json(serde_json::json!({
        "flat": s.flat,
        "status": "raw-only",
        "revision": manifest.as_ref().and_then(|m| m.get("revision_id")),
        "asset": {
            "format": "usdz",
            "byte_length": bytes.len(),
            "sha256": sha256,
            "url": "/interior/api/roomplan/asset"
        },
        "observation": observation,
        "manifest": manifest,
    })))
}

async fn api_roomplan_asset(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (_, asset, _) = latest_roomplan_capture(&s.flat).await?;
    let bytes = tokio::fs::read(asset).await.map_err(boom)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "model/vnd.usdz+zip")],
        bytes,
    ))
}

async fn roomplan_revision_drafts(
    flat: &str,
) -> Result<Vec<serde_json::Value>, (StatusCode, String)> {
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(flat);
    if !tokio::fs::try_exists(&root).await.map_err(boom)? {
        return Ok(Vec::new());
    }
    let mut dates = tokio::fs::read_dir(&root).await.map_err(boom)?;
    let mut drafts = Vec::new();
    while let Some(date) = dates.next_entry().await.map_err(boom)? {
        if !date.file_type().await.map_err(boom)?.is_dir() {
            continue;
        }
        let direct = date.path().join("draft.json");
        if tokio::fs::try_exists(&direct).await.map_err(boom)? {
            drafts.push(read_json_file(&direct).await?);
        }
        let revisions = date.path().join("revisions");
        if !tokio::fs::try_exists(&revisions).await.map_err(boom)? {
            continue;
        }
        let mut entries = tokio::fs::read_dir(revisions).await.map_err(boom)?;
        while let Some(entry) = entries.next_entry().await.map_err(boom)? {
            if !entry.file_type().await.map_err(boom)?.is_dir() {
                continue;
            }
            let draft = entry.path().join("draft.json");
            if tokio::fs::try_exists(&draft).await.map_err(boom)? {
                drafts.push(read_json_file(&draft).await?);
            }
        }
    }
    drafts.sort_by(|left, right| {
        left.get("created_at")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .cmp(
                right
                    .get("created_at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            )
    });
    Ok(drafts)
}

async fn read_json_file(path: &std::path::Path) -> Result<serde_json::Value, (StatusCode, String)> {
    serde_json::from_slice(&tokio::fs::read(path).await.map_err(boom)?)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn api_roomplan_revisions(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    Ok(Json(serde_json::json!({
        "flat": s.flat,
        "revisions": roomplan_revision_drafts(&s.flat).await?,
    })))
}

#[derive(Debug, Deserialize)]
struct RoomPlanReviewRequest {
    decision: String,
    #[serde(default)]
    #[serde(alias = "notiz")]
    note: Option<String>,
}

async fn api_roomplan_review(
    State(s): State<Arc<AppState>>,
    Path(revision_id): Path<String>,
    Json(request): Json<RoomPlanReviewRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !matches!(request.decision.as_str(), "accept" | "reject") {
        return Err((
            StatusCode::BAD_REQUEST,
            "decision must be accept or reject".to_string(),
        ));
    }
    let root = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("captures")
        .join(&s.flat);
    let mut dates = tokio::fs::read_dir(&root).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            "keine RoomPlan-Aufnahmen".to_string(),
        )
    })?;
    while let Some(date) = dates.next_entry().await.map_err(boom)? {
        if !date.file_type().await.map_err(boom)?.is_dir() {
            continue;
        }
        let mut candidates = Vec::new();
        let direct = date.path().join("draft.json");
        if tokio::fs::try_exists(&direct).await.map_err(boom)? {
            candidates.push(direct);
        }
        let revisions = date.path().join("revisions");
        if tokio::fs::try_exists(&revisions).await.map_err(boom)? {
            let mut entries = tokio::fs::read_dir(revisions).await.map_err(boom)?;
            while let Some(entry) = entries.next_entry().await.map_err(boom)? {
                if entry.file_type().await.map_err(boom)?.is_dir() {
                    candidates.push(entry.path().join("draft.json"));
                }
            }
        }
        for draft_path in candidates {
            if !tokio::fs::try_exists(&draft_path).await.map_err(boom)? {
                continue;
            }
            let draft = read_json_file(&draft_path).await?;
            if draft.get("draft_id").and_then(serde_json::Value::as_str)
                != Some(revision_id.as_str())
            {
                continue;
            }
            let review = serde_json::json!({
                "decision": request.decision,
                "note": request.note,
            });
            let review_path = draft_path.with_file_name("review.json");
            let bytes = serde_json::to_vec_pretty(&review)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            tokio::fs::write(review_path, bytes).await.map_err(boom)?;
            return Ok(Json(review));
        }
    }
    Err((
        StatusCode::NOT_FOUND,
        "RoomPlan-Revision nicht gefunden".to_string(),
    ))
}

async fn api_media(Path(pfad): Path<String>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let wurzel = crate::model::assets_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .join("media");
    let wurzel = tokio::fs::canonicalize(&wurzel)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "kein media-Verzeichnis".to_string()))?;
    let target = tokio::fs::canonicalize(wurzel.join(&pfad))
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("kein Medium `{pfad}`")))?;
    let ist_datei = tokio::fs::metadata(&target)
        .await
        .map(|m| m.is_file())
        .unwrap_or(false);
    if !target.starts_with(&wurzel) || !ist_datei {
        return Err((
            StatusCode::FORBIDDEN,
            format!("`{pfad}` liegt nicht unter media/"),
        ));
    }
    let typ = match target
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        // Kein Standardtyp: was hier unbekannt ist, wird nicht geraten und nicht geliefert.
        _ => return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "kein Bildformat".into())),
    };
    let bytes = tokio::fs::read(&target).await.map_err(boom)?;
    Ok(([(axum::http::header::CONTENT_TYPE, typ)], bytes))
}

/// Wo die linke obere Ecke eines Stuecks liegen darf — harte Kanten fuers Ziehen.
///
/// Die Oberflaeche fragt einmal beim Aufnehmen und rastet danach auf die Liste ein. Sie
/// bekommt Lauflaengen und keine Geometrie: der Hauptraum ist ein Sechseck, und auf ein
/// umschliessendes Rechteck zu klemmen wuerde ein Moebel in der Kerbe abstellen, in der das
/// Bad liegt.
async fn api_allowed(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
    axum::extract::Query(q): axum::extract::Query<AllowedQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let base = model
        .load_layout(&name)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let a = crate::search::allowed_positions(&model, &base, &q.reference, q.rot)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(a))
}

#[derive(serde::Deserialize)]
struct AllowedQuery {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    rot: i32,
}

/// Der Ist-Zustand als Aufstellung — die Vorlage kommt im Rumpf und nicht aus einer Datei.
///
/// Der Name ist die einzige Stelle, an der sich das vom Layout unterscheidet, und deshalb wird
/// hier nichts zusammengelegt: `/api/layouts/{name}/preview` **liest** eine Entscheidung, samt
/// dem Kopf der Datei, in dem steht, warum ein Stueck so steht (PRD Q60). Eine Platzierung hat
/// keinen Kopf — sie ist der Zustand und eine Zeile, nicht der Vorschlag und eine Datei. Zwei
/// Endpunkte, weil zwei Dinge hereinkommen, und nicht weil zwei Dinge herauskommen: Plan und
/// Verdikt sind dieselben und werden von derselben Funktion gerechnet.
fn as_is(items: Vec<crate::model::PlacedItem>) -> crate::model::Layout {
    crate::model::Layout {
        name: "ist-zustand".to_string(),
        id: "ist-zustand".to_string(),
        items,
    }
}

async fn api_preview_placements(
    State(s): State<Arc<AppState>>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = as_is(body.items);
    let r = check_layout(&model, &l).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

/// Erlaubte Positionen eines Stuecks im Ist-Zustand.
///
/// Die Aufstellung kommt im Rumpf, weil sie nirgends sonst steht: die Platzierungen sind Zeilen
/// in der Datenbank und keine Datei, die man nachladen koennte. `allowed_positions` braucht sie
/// als Grundlage, weil ein Stueck nicht dort stehen darf, wo ein anderes schon steht.
async fn api_placements_allowed(
    State(s): State<Arc<AppState>>,
    Json(body): Json<PlacementsAllowedBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let base = as_is(body.items);
    let a = crate::search::allowed_positions(&model, &base, &body.reference, body.rot)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(a))
}

#[derive(serde::Deserialize)]
struct PlacementsAllowedBody {
    items: Vec<crate::model::PlacedItem>,
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    rot: i32,
}

/// Wie ein Layout AUSSAEHE und ausfiele, ohne es zu schreiben.
///
/// Verschieben kann die Oberflaeche selbst zeichnen: eine Verschiebung ist eine Translation und
/// aendert an der Grundflaeche nichts. **Drehen kann sie nicht** — bei 90 Grad tauschen Breite
/// und Tiefe, und `opens` und `expands_dir` drehen mit. Das im Browser nachzubauen waere eine
/// zweite Fassung von `footprint` und `Side::gedreht`, also genau die Doppelung, gegen die
/// diese Capability existiert. Sie fragt stattdessen hier nach.
async fn api_preview_layout(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let template = model
        .load_layout(&id)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let l = crate::model::Layout {
        name: template.name,
        items: body.items,
        id: id.clone(),
    };
    let r = check_layout(&model, &l).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok(Json(
        serde_json::json!({ "layout": l, "check": r, "svg": svg }),
    ))
}

/// Ein neues Layout anlegen. Ueberschreibt nie ein bestehendes.
///
/// Drei Wege in einen Rumpf, weil es drei Arten gibt, einen Plan anzufangen, und keine davon
/// eine Sonderform verdient: **leer** (weder `von` noch `items`), **als Kopie** (`von`), oder
/// **fertig gestellt** (`items`). Ein Aufrufer, der eine Variante durchspielen will, schickt
/// eine Zeile und faengt nicht damit an, sich eine Aufstellung aus `GET` zusammenzusuchen.
///
/// Antwortet wie `PUT` und die Vorschau mit `{layout, check, svg}`: wer einen Plan anlegt, will
/// als Naechstes wissen, ob er besteht und wie er aussieht, und das ist dieselbe Rechnung.
async fn api_post_layout(
    State(s): State<Arc<AppState>>,
    Json(body): Json<NewLayout>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    // Ein unbrauchbarer Name ist eine schlechte Anfrage und kein Konflikt. Beides auf 409 zu
    // legen hiesse, einem Aufrufer „gibt es schon" zu melden, wo „so darf es nicht heissen"
    // gemeint ist.
    crate::layout_io::check_id(&body.id).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let template = match &body.von {
        // Der Name der Vorlage und nicht der Lesefehler: der traegt den Pfad im privaten
        // Overlay, und der Aufrufer hat nach einem Layout gefragt, nicht nach einer Datei.
        Some(v) => Some(
            model
                .load_layout(v)
                .map_err(|_| (StatusCode::NOT_FOUND, format!("keine Vorlage `{v}`")))?,
        ),
        None => None,
    };
    let items = match (body.items, &template) {
        (Some(own), _) => own,
        (None, Some(v)) => v.items.clone(),
        (None, None) => Vec::new(),
    };
    let notiz = body.notiz.unwrap_or_else(|| match &body.von {
        Some(v) => format!(
            "Erstellt {} ueber die API, als Kopie von `{v}`.",
            crate::layout_io::today()
        ),
        None => format!("Erstellt {} ueber die API.", crate::layout_io::today()),
    });
    let layout = crate::model::Layout {
        name: body.name.unwrap_or_else(|| body.id.clone()),
        items,
        id: body.id.clone(),
    };
    crate::layout_io::create(&model, &body.id, &layout, &notiz)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    let l = model
        .load_layout(&body.id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let r = check_layout(&model, &l).map_err(boom)?;
    let svg = plan::svg(&model, &l).map_err(boom)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": body.id, "layout": l, "check": r, "svg": svg })),
    ))
}

#[derive(serde::Deserialize)]
struct NewLayout {
    id: String,
    /// Der Anzeigename. Fehlt er, ist es die Id — ein Plan ohne Namen waere in jeder Liste eine
    /// leere Zeile, und ein erfundener waere schlimmer.
    #[serde(default)]
    name: Option<String>,
    /// Die Vorlage, deren Aufstellung uebernommen wird.
    #[serde(default)]
    von: Option<String>,
    /// Schlaegt `von`. Fehlen beide, entsteht ein **leerer** Plan — kein Versehen, sondern der
    /// Anfang jeder Planung, die von Hand gezogen wird.
    #[serde(default)]
    items: Option<Vec<crate::model::PlacedItem>>,
    /// Die erste Zeile des Dateikopfs, und damit die Begruendung. Ohne sie schreibt der Kopf
    /// das Datum und die API hin, statt eine Herkunft zu behaupten, die niemand hat.
    #[serde(default)]
    notiz: Option<String>,
}

/// Ein Layout aus der Liste nehmen, ohne es zu verlieren.
///
/// `DELETE` heisst hier **archivieren**: die Datei wandert nach `layouts/archiv/` und bleibt
/// lesbar. Ihr Kopf traegt die Begruendung, aus der ein Moebel verworfen wurde (PRD Q60), und
/// die wird genau dann gebraucht, wenn dasselbe Moebel wieder zur Debatte steht.
async fn api_delete_layout(
    State(s): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    crate::layout_io::check_id(&id).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // Ueber die Namensliste und nicht ueber `load_layout`: die liest die Datei, und ein Layout
    // mit einem Tippfehler im TOML ist genau eins, das jemand wegraeumen will.
    if !model.layout_names().map_err(boom)?.iter().any(|n| n == &id) {
        return Err((StatusCode::NOT_FOUND, format!("kein Layout `{id}`")));
    }
    let nach = crate::layout_io::archive_layout(&model, &id)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "id": id, "archiviert": nach, "geloescht": false }),
    ))
}

/// Wo die Stuecke in dieser Wohnung WIRKLICH stehen — nicht, was vorgeschlagen ist.
///
/// Der Unterschied ist der Grund, aus dem `interior_placement` eine eigene Tabelle ist: ein
/// Layout ist ein Vorschlag und eine Datei, eine Platzierung ist der Zustand und eine Zeile.
/// Die Tabelle stand seit B25 leer, weil nichts sie geschrieben hat.
async fn api_put_placements(
    State(s): State<Arc<AppState>>,
    Json(body): Json<LayoutBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let st = store(&s)?;
    for it in &body.items {
        st.place(&crate::store::Placement {
            item_id: it.reference.clone(),
            flat: s.flat.clone(),
            x: it.x,
            y: it.y,
            rot: it.rot,
        })
        .map_err(boom)?;
    }
    Ok(Json(
        serde_json::json!({ "flat": s.flat, "gesetzt": body.items.len() }),
    ))
}

async fn index(State(s): State<Arc<AppState>>) -> Result<Html<String>, (StatusCode, String)> {
    let m = load(&s)?;
    let names = m
        .layout_names()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let layouts: Vec<_> = names.iter().filter_map(|n| m.load_layout(n).ok()).collect();
    plan::page(&m, &layouts, plan::Origin::Proposal)
        .map(Html)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn serve(flat: &str, port: u16) {
    let state = Arc::new(AppState {
        flat: flat.to_string(),
        database: sjel_config::database_path(),
    });
    sjel_server::serve_local("interior", port, build_router(state)).await;
}

/// Der Name dieser Capability, fuer die Umgebungsvariable der Origin-Sperre
/// (`SJEL_INTERIOR_ALLOWED_ORIGIN_HOSTS`).
const CAPABILITY: &str = "interior";

/// Der verdrahtete Router, damit ein Test das echte Ding fahren kann statt einen Handler.
///
/// Diese Capability traegt keine CORS-Schicht, also kann eine fremde Side die Antwort
/// nicht lesen — und genau das hat eine Luecke verdeckt, die bis 2026-10-05 hier stand:
/// `POST /api/vault/writeback` nahm keinen Request-Body, war damit eine *einfache* Anfrage
/// im Sinne von CORS, lief ohne Preflight und schrieb in die Obsidian-Vault. Ob der Browser
/// die Antwort danach weiterreicht, ist fuer einen Schreibvorgang gleichgueltig; er ist
/// schon passiert. Diese Route ist mit der Item-Oberflaeche nach `capabilities/inventory`
/// gezogen — und hat ihre Sperre mitgenommen. Hier bleibt sie, weil jeder Handler dieser
/// Capability privates liefert: einen fremden Ursprung gar nicht erst bis zum Handler zu
/// lassen ist die billigere Antwort als eine, die nur keinen Header zurueckschickt.
///
/// Die Sperre liegt absichtlich unter allen Routen: axum umhuellt nur die Routen, die VOR
/// einem `.layer()`-Aufruf registriert wurden (axum 0.7 `src/docs/routing/layer.md`), eine
/// darunter angehaengte Route verloere sie stillschweigend.
fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/routes", get(routes))
        .route("/api/model", get(api_model))
        .route("/api/layouts", get(api_layouts).post(api_post_layout))
        .route(
            "/api/layouts/{name}",
            get(api_layout)
                .put(api_put_layout)
                .delete(api_delete_layout),
        )
        .route("/api/layouts/{name}/preview", post(api_preview_layout))
        .route("/api/layouts/{name}/allowed", get(api_allowed))
        .route("/api/placements", put(api_put_placements))
        .route("/api/placements/preview", post(api_preview_placements))
        .route("/api/placements/allowed", post(api_placements_allowed))
        .route("/api/flats", get(api_flats))
        .route("/api/media/{*pfad}", get(api_media))
        .route("/api/roomplan/reference", get(api_roomplan_reference))
        .route("/api/roomplan/asset", get(api_roomplan_asset))
        .route("/api/roomplan/revisions", get(api_roomplan_revisions))
        .route(
            "/api/roomplan/revisions/{revision_id}/review",
            post(api_roomplan_review),
        )
        .route("/api/placements/{flat}", get(api_placements))
        // Die Item-Oberflaeche liegt in `capabilities/inventory` (ISA F13). Hier bleibt
        // genau eine Item-Route: die Vorschau, was eine Aenderung mit den Verdikten machen
        // wuerde. Die rechnet gegen Raeumungsregeln und Layouts, also gehoert sie hierher —
        // und sie schreibt nichts.
        .route("/api/items/{id}/impact", post(api_item_impact))
        .route("/api/placements/{flat}/{item}", put(api_put_placement))
        .route("/api/layouts/{name}/toleranz", get(api_tolerance))
        .route("/api/layouts/{name}/sonne", get(api_sun))
        .route("/api/layouts/{name}/einbringung", get(api_delivery))
        .route("/api/passt", get(api_fits))
        .route("/api/deklaration", get(api_declarations))
        .route("/api/kaufen", get(api_purchase_order))
        .route("/api/search", post(api_search))
        .route("/api/compose", post(api_compose))
        .route("/api/auftraege/{id}", get(api_job))
        // NEUE ROUTEN UEBER DIESE ZEILE. Darunter verlieren sie die Origin-Sperre.
        .layer(axum::middleware::from_fn_with_state(
            CAPABILITY,
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

#[cfg(test)]
mod route_manifest_tests {
    /// Ein Katalog, der luegt, wird geglaubt. Das hier liest den Router aus seinem eigenen
    /// Quelltext, also faellt ein `.route()` ohne Zusammenfassung hier um, statt eine
    /// Oberflaeche auszuliefern, die sich selbst falsch beschreibt.
    #[test]
    fn the_catalogue_names_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("api.rs"), super::ROUTES);
        assert!(
            missing.is_empty(),
            "ausgeliefert, aber nicht beschrieben: {missing:?}"
        );
    }
}

/// Der Beweis auf Router-Ebene, den die Praedikat-Tests in `libs/sjel-server` nicht fuehren
/// koennen: eine Route UNTER dem `.layer()`-Aufruf besteht jeden Test von
/// `origin_allowed_by` und antwortet einer fremden Side trotzdem.
///
/// `tower::ServiceExt::oneshot` statt eines Loopback-Listeners, damit der Test weder einen
/// Port noch einen HTTP-Client braucht.
///
/// `/routes` ist die Gegenprobe und nicht `/api/inventory`: jeder Datenhandler hier oeffnet
/// die SQLite-Datei der Installation, und das darf ein Test nicht. Die Zurueckweisung wird
/// auf den Datenrouten geprueft, wo die Sperre vor dem Handler antwortet und nichts oeffnet.
#[cfg(test)]
mod origin_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn answer(methode: &str, pfad: &str, origin: Option<&str>) -> StatusCode {
        let mut anfrage = Request::builder().method(methode).uri(pfad);
        if let Some(origin) = origin {
            anfrage = anfrage.header("origin", origin);
        }
        build_router(Arc::new(AppState {
            flat: "flat".to_string(),
            database: sjel_config::database_path(),
        }))
        .oneshot(anfrage.body(Body::empty()).unwrap())
        .await
        .expect("der Router antwortet")
        .status()
    }

    /// Jeder Datenhandler hier liefert privates — einen Grundriss, private Fotos, die
    /// Verdikte einer Wohnung. Genau deshalb weist die Sperre die Anfrage zurueck, statt nur
    /// einen Antwort-Header wegzulassen.
    #[tokio::test]
    async fn a_foreign_page_reaches_no_data_handler() {
        for (methode, pfad) in [
            ("GET", "/api/model"),
            ("GET", "/api/layouts"),
            ("GET", "/api/deklaration"),
            ("POST", "/api/search"),
            ("POST", "/api/items/schrank/impact"),
        ] {
            assert_eq!(
                answer(methode, pfad, Some("https://evil.example")).await,
                StatusCode::FORBIDDEN,
                "{methode} {pfad} hat einer fremden Herkunft geantwortet — die Route steht unter der Sperre"
            );
        }
    }

    /// Die andere Haelfte. 200 von `/routes` heisst: ein Handler hat geantwortet.
    #[tokio::test]
    async fn the_dashboard_and_a_non_browser_caller_reach_the_handler() {
        for origin in [
            None,
            Some("http://localhost:47117"),
            Some("https://mac.tailnet.ts.net"),
        ] {
            assert_eq!(
                answer("GET", "/routes", origin).await,
                StatusCode::OK,
                "die Sperre hat einen Aufrufer zurueckgewiesen, den sie zulassen muss: {origin:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Jobs, JobState, JOBS_MAX};
    use serde_json::json;

    /// Zehn Auftraege anlegen und die jungen davon fertig melden.
    fn full_map(run: u64) -> Jobs {
        let mut a = Jobs::default();
        for _ in 0..JOBS_MAX {
            a.anlegen().expect("unter der Grenze");
        }
        for id in run..JOBS_MAX as u64 {
            a.stand.get_mut(&id).expect("gerade angelegt").1 = JobState::Done {
                outcome: json!(null),
            };
        }
        a
    }

    /// Verdraengt wird der aelteste FERTIGE, nicht der aelteste.
    ///
    /// Bis 2026-08-31 warf `anlegen` blind den ersten Schluessel weg. Traf es einen, der noch
    /// rechnete, schrieb sein Hintergrundfaden das Ergebnis in eine Nummer, die es nicht mehr
    /// gab — und der Abholer bekam 404 auf eine Suche, die Minuten gelaufen war.
    #[test]
    fn a_running_job_is_not_evicted() {
        let mut a = full_map(1);
        let neu = a.anlegen().expect("neun fertige machen Platz");
        assert!(a.stand.contains_key(&0), "der laufende steht noch da");
        assert!(
            !a.stand.contains_key(&1),
            "der aelteste fertige ist gewichen"
        );
        assert!(a.stand.contains_key(&neu));
        assert_eq!(a.stand.len(), JOBS_MAX);
    }

    /// Rechnen alle zehn, ist die Absage die einzige ehrliche Antwort.
    #[test]
    fn a_map_full_of_running_jobs_refuses() {
        let mut a = full_map(JOBS_MAX as u64);
        let reason = a.anlegen().expect_err("es gibt nichts zu verdraengen");
        assert!(reason.contains("rechnen bereits"), "{reason}");
        assert_eq!(a.stand.len(), JOBS_MAX, "und keiner ist verschwunden");
        assert!((0..JOBS_MAX as u64).all(|id| a.stand.contains_key(&id)));
    }

    /// Eine einzige Panik unter der Sperre beendete bis hierher die drei Wege, die
    /// die Karte anfassen: `expect("Auftragskarte")` auf einem vergifteten Schloss
    /// ist die zweite Panik, und `map_err(boom)` im Abholer waere 500 fuer die
    /// Lebensdauer des Prozesses gewesen.
    ///
    /// Das vergiftet die echte prozessweite Karte mit Absicht. Dass jeder andere
    /// Test in dieser Binaerdatei danach weiterlaeuft, ist die Zusage unter den
    /// ausgeschriebenen.
    #[test]
    fn a_poisoned_job_map_still_creates_and_reads() {
        let poison = std::thread::spawn(|| {
            let _lock = super::jobs();
            panic!("jemand ist mit der Auftragskarte in der Hand gestuerzt");
        })
        .join();
        assert!(poison.is_err(), "der Hilfsfaden muss wirklich stuerzen");

        let id = super::jobs()
            .anlegen()
            .expect("die Karte legt weiter an");
        assert!(
            super::jobs().stand.contains_key(&id),
            "eine erholte Sperre muss den Eintrag noch zeigen"
        );
    }
}

// ---------------------------------------------------------------- lange Rechnungen

/// Was aus einer Suche geworden ist.
///
/// **Warum das ueberhaupt eine eigene Form braucht.** `search` prueft die Kandidaten
/// erschoepfend — 3,5 Millionen in rund hundert Sekunden (PRD §13.1). Eine HTTP-Anfrage, die
/// hundert Sekunden offen steht, ist keine Anfrage mehr, sondern eine Wette auf jeden Proxy
/// und jedes Zeitlimit dazwischen. Bis 2026-08-31 gab es die Suche deshalb nur auf der
/// Kommandozeile: die teuerste Rechnung dieser Capability war von der Oberflaeche aus, die sie
/// braucht, nicht erreichbar.
///
/// Der Auftrag ist die Antwort darauf, und er ist absichtlich das kleinste, was funktioniert:
/// eine Nummer, ein Zustand, ein Ergebnis. Keine Warteschlange, keine Wiederaufnahme, keine
/// Tabelle. Er lebt im Prozess und stirbt mit ihm — was richtig ist, weil sein Ergebnis eine
/// Liste von Vorschlaegen ist und keine Tatsache ueber die Wohnung. Wer den Vorschlag behalten
/// will, schreibt ihn als Layout.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "zustand", rename_all = "snake_case")]
pub enum JobState {
    Running { seit_ms: u128 },
    Done { outcome: serde_json::Value },
    Failed { reason: String },
}

#[derive(Default)]
struct Jobs {
    nearest: u64,
    stand: std::collections::BTreeMap<u64, (std::time::Instant, JobState)>,
}

/// Wie viele fertige Auftraege aufgehoben werden.
///
/// Ohne Grenze waechst die Karte mit jeder Suche, und ein Prozess, der wochenlang laeuft,
/// haelt jedes Ergebnis fest, das je jemand angesehen hat. Zehn, weil die Oberflaeche das
/// letzte abholt und die davor nur noch Verlauf sind.
const JOBS_MAX: usize = 10;

impl Jobs {
    /// Platz schaffen und die naechste Nummer ziehen — oder ablehnen.
    ///
    /// Verdraengt wird nur, was schon fertig ist. Ein laufender Auftrag hat seinen Schreiber
    /// noch im Hintergrund: verschwindet der Eintrag, schreibt der in nichts, und der Abholer
    /// bekommt 404 auf eine Rechnung, die minutenlang lief. Sind alle zehn belegt, ist die
    /// ehrliche Antwort eine Absage und keine stillschweigend verlorene Suche.
    fn anlegen(&mut self) -> Result<u64, String> {
        while self.stand.len() >= JOBS_MAX {
            // Die Schluessel steigen, also ist der erste verdraengbare auch der aelteste.
            let Some(alt) = self
                .stand
                .iter()
                .find(|(_, (_, s))| !matches!(s, JobState::Running { .. }))
                .map(|(id, _)| *id)
            else {
                return Err(format!(
                    "{JOBS_MAX} Auftraege rechnen bereits — warte, bis einer fertig ist"
                ));
            };
            self.stand.remove(&alt);
        }
        let id = self.nearest;
        self.nearest += 1;
        self.stand.insert(
            id,
            (
                std::time::Instant::now(),
                JobState::Running { seit_ms: 0 },
            ),
        );
        Ok(id)
    }
}

/// Die Auftragskarte, gesperrt, mit einem vergifteten Schloss erholt statt weitergereicht.
///
/// Sie gibt die Sperre zurueck und nicht den `Mutex`: drei Aufrufstellen schrieben je eine
/// eigene Behandlung — zweimal `expect("Auftragskarte")`, einmal `map_err(boom)` — und eine
/// Entscheidung, die an jeder Aufrufstelle wiederholt wird, faellt an der vierten anders aus.
/// Jetzt sperrt nur diese Stelle, also muss auch nur diese Stelle stimmen.
///
/// **Erholen, nicht toedlich.** Vergiftet ist das Schloss, wenn jemand mit der Sperre in der
/// Hand in Panik geraten ist. Unter der Sperre liegt nichts Dauerhaftes: `Auftraege` ist eine
/// `BTreeMap` von Nummern auf Zustaende im Prozess, sicheres Rust kann sie nicht halb
/// beschrieben hinterlassen, und der schlechteste Fall ist ein Eintrag, der `Laeuft` sagt,
/// waehrend sein Faden weg ist — genau der Zustand, den `anlegen` schon kennt und nicht
/// verdraengt.
///
/// Die Kosten der Gegenrichtung sind das Argument. Ein `expect` auf ein vergiftetes Schloss
/// ist eine zweite Panik, also wuerden `POST /api/search`, `POST /api/compose` und
/// `GET /api/auftraege/{id}` fuer die Lebensdauer des Prozesses umfallen, weil irgendwann
/// einmal jemand mit der Sperre gestuerzt ist — und das Ergebnis einer Suche, die Minuten
/// gelaufen ist, waere unerreichbar. Dieselbe Wahl trifft
/// `capabilities/scouting/src/config.rs`' `env_lock` mit derselben Begruendung.
///
/// Was das ausdruecklich NICHT tut: die erste Panik verstecken. Die laeuft weiter auf, landet
/// in der Standardfehlerausgabe des Runners und bleibt das, was man liest.
fn jobs() -> std::sync::MutexGuard<'static, Jobs> {
    static A: std::sync::OnceLock<std::sync::Mutex<Jobs>> = std::sync::OnceLock::new();
    A.get_or_init(|| std::sync::Mutex::new(Jobs::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Eine Rechnung im Hintergrund starten und sofort ihre Nummer zurueckgeben.
///
/// `spawn_blocking`, weil `search` und `compose` rayon benutzen und minutenlang rechnen: auf
/// einem async-Thread wuerde das jede andere Anfrage dieses Prozesses anhalten. Dieselbe
/// Begruendung, aus der `punctuality` und `finance` ihre schweren Wege dorthin legen (PRD B20).
fn in_the_background<F>(f: F) -> Result<u64, (StatusCode, String)>
where
    F: FnOnce() -> Result<serde_json::Value, String> + Send + 'static,
{
    let id = jobs()
        .anlegen()
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
    tokio::task::spawn_blocking(move || {
        let outcome = f();
        let mut a = jobs();
        if let Some((_, stand)) = a.stand.get_mut(&id) {
            *stand = match outcome {
                Ok(v) => JobState::Done { outcome: v },
                Err(e) => JobState::Failed { reason: e },
            };
        }
    });
    Ok(id)
}

async fn api_job(Path(id): Path<u64>) -> Result<impl IntoResponse, (StatusCode, String)> {
    let a = jobs();
    let (start, stand) = a
        .stand
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("kein Auftrag {id}")))?;
    // Die Laufzeit wird beim Lesen gerechnet und nicht fortgeschrieben: ein Auftrag, der sie
    // selbst zaehlen muesste, braeuchte einen zweiten Faden fuer nichts.
    let stand = match stand {
        JobState::Running { .. } => JobState::Running {
            seit_ms: start.elapsed().as_millis(),
        },
        done => done.clone(),
    };
    Ok(Json(serde_json::json!({ "id": id, "stand": stand })))
}

#[derive(serde::Deserialize)]
struct SearchRequest {
    layout: String,
    #[serde(default)]
    move_refs: Vec<String>,
    #[serde(default = "raster_standard")]
    step: i32,
    #[serde(default = "search_limit")]
    limit: usize,
}

fn raster_standard() -> i32 {
    20
}
/// Dieselbe Zahl wie `--limit` auf der Kommandozeile (`main.rs`, `cmd_search`). Ohne sie stand
/// hier `usize::default()`, und die Null heisst in `search::search` *unbegrenzt*: dieselbe
/// Anfrage lieferte auf der Oberflaeche Tausende Treffer und im Terminal sechs.
fn search_limit() -> usize {
    6
}

/// Eine Suche anstossen. Antwortet mit der Auftragsnummer, nicht mit dem Ergebnis.
async fn api_search(
    State(s): State<Arc<AppState>>,
    Json(anfrage): Json<SearchRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if anfrage.move_refs.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "`move_refs` ist leer — ohne bewegliche Moebel gibt es nichts zu suchen".into(),
        ));
    }
    let flat = s.flat.clone();
    let id = in_the_background(move || {
        let model = Model::load(&flat).map_err(|e| e.to_string())?;
        let base = model
            .load_layout(&anfrage.layout)
            .map_err(|e| e.to_string())?;
        let spec = crate::search::Spec {
            move_refs: anfrage.move_refs,
            step: anfrage.step,
            bands: Default::default(),
            limit: anfrage.limit,
        };
        let rep = crate::search::search(&model, &base, &spec).map_err(|e| e.to_string())?;
        serde_json::to_value(rep).map_err(|e| e.to_string())
    })?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "auftrag": id })),
    ))
}

#[derive(serde::Deserialize)]
struct ComposeRequest {
    refs: Vec<String>,
    #[serde(default = "compose_raster")]
    step: i32,
    #[serde(default = "compose_beam")]
    beam: usize,
    #[serde(default = "compose_limit")]
    limit: usize,
    #[serde(default = "compose_rotations")]
    rotations: Vec<i32>,
}

fn compose_raster() -> i32 {
    25
}
fn compose_beam() -> usize {
    60
}
/// Dieselbe Zahl wie `--limit` auf der Kommandozeile (`main.rs`, `cmd_compose`).
fn compose_limit() -> usize {
    5
}
fn compose_rotations() -> Vec<i32> {
    vec![0, 90]
}

/// Eine ganze Wohnung stellen lassen. Ebenfalls ein Auftrag: die Strahlsuche rechnet Minuten.
async fn api_compose(
    State(s): State<Arc<AppState>>,
    Json(anfrage): Json<ComposeRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if anfrage.refs.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "`refs` ist leer — ohne Stuecke gibt es nichts zu stellen".into(),
        ));
    }
    let flat = s.flat.clone();
    let id = in_the_background(move || {
        let model = Model::load(&flat).map_err(|e| e.to_string())?;
        let spec = crate::search::ComposeSpec {
            refs: anfrage.refs,
            step: anfrage.step,
            beam: anfrage.beam,
            rotations: anfrage.rotations,
            limit: anfrage.limit,
        };
        let out = crate::search::compose(&model, &spec).map_err(|e| e.to_string())?;
        serde_json::to_value(out).map_err(|e| e.to_string())
    })?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "auftrag": id })),
    ))
}

// ---------------------------------------------------------------- die neuen Auskuenfte

async fn api_tolerance(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(not_found)?;
    Ok(Json(crate::tolerance::robustness(&model, &l).map_err(boom)?))
}

async fn api_sun(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(not_found)?;
    Ok(Json(crate::sun::report(&model, &l).map_err(boom)?))
}

async fn api_delivery(
    State(s): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let l = model.load_layout(&name).map_err(not_found)?;
    let mut out = Vec::new();
    for it in &l.items {
        out.push(crate::delivery::delivery(&model, &l, &it.reference).map_err(boom)?);
    }
    Ok(Json(out))
}

#[derive(serde::Deserialize)]
struct PieceSize {
    b: i32,
    t: i32,
    #[serde(default)]
    #[serde(alias = "zerlegbar")]
    disassembles: bool,
}

/// Passt ein gedachtes Stueck durch die Tuer? Die Frage VOR dem Kauf.
async fn api_fits(
    State(s): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<PieceSize>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    Ok(Json(crate::delivery::through_the_door(
        &model,
        q.b,
        q.t,
        q.disassembles,
    )))
}

async fn api_declarations(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    Ok(Json(crate::declarations::overview(&model).map_err(boom)?))
}

async fn api_purchase_order(
    State(s): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = load(&s)?;
    let st = store(&s)?;
    let conn = st.borrow_connection().map_err(boom)?;
    let balance = crate::budget::monthly_balance(&conn).map_err(boom)?;
    Ok(Json(
        crate::budget::purchase_order(&model, balance).map_err(boom)?,
    ))
}

fn not_found(e: crate::model::ModelError) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, e.to_string())
}
