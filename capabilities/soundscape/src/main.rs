//! soundscape — the conductor.
//!
//! Web Audio runs in the browser and stays there: this process cannot generate the
//! sound, and rendering audio here to stream it back would be a worse system for no
//! gain. What it owns is *what should be playing* — preset, parameters, layer mix,
//! seed — so that the answer survives a reload, is the same on every surface, and
//! can be changed from a phone without walking to the desk.
//!
//! What it deliberately does NOT own is the clock. The arrangement phase is a
//! function of the audio context's own time, and the browser is where that time
//! exists; publishing a phase from here would put two clocks in disagreement over
//! the same bar. The conductor owns intent, the browser owns timing.

use std::sync::{Arc, RwLock};

use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        Json,
    },
    routing::get,
    Router,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use tower_http::services::{ServeDir, ServeFile};

const PRESETS: [&str; 6] = ["edm", "ambient", "lofi", "focus", "relax", "sleep"];
const SCENARIOS: [&str; 5] = ["deep-work", "reading", "reset", "wind-down", "timer"];
const MAX_SESSION_MS: u64 = 8 * 60 * 60 * 1000;

/// The six scene parameters, same names and range as the browser engine's. They
/// live here rather than in the UI because a second surface has to be able to read
/// what the first one set.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Params {
    pace: f32,
    density: f32,
    brightness: f32,
    space: f32,
    pulse: f32,
    texture: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Layers {
    drums: f32,
    bass: f32,
    pads: f32,
    melody: f32,
    texture: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Session {
    scenario: String,
    duration_ms: u64,
    elapsed_ms: u64,
    running_since_ms: Option<u64>,
}

impl Session {
    fn remaining_at(&self, now: u64) -> u64 {
        let running = self
            .running_since_ms
            .map(|started| now.saturating_sub(started))
            .unwrap_or(0);
        self.duration_ms
            .saturating_sub(self.elapsed_ms.saturating_add(running))
    }

    fn is_valid(&self, now: u64) -> bool {
        SCENARIOS.contains(&self.scenario.as_str())
            && self.duration_ms > 0
            && self.duration_ms <= MAX_SESSION_MS
            && self.elapsed_ms <= self.duration_ms
            && self
                .running_since_ms
                .is_none_or(|started| started <= now.saturating_add(60_000))
    }

    fn pause_at(&mut self, now: u64) {
        if let Some(started) = self.running_since_ms.take() {
            self.elapsed_ms = self
                .duration_ms
                .min(self.elapsed_ms.saturating_add(now.saturating_sub(started)));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Scape {
    preset: String,
    params: Params,
    layers: Layers,
    seed: u32,
    playing: bool,
    volume: f32,
    energy: f32,
    #[serde(default)]
    session: Option<Session>,
}

impl Default for Scape {
    /// Mirrors the browser engine's own defaults. Two sources of truth for a
    /// default is one too many, but until the param shape moves into schemas/
    /// (CONTRIBUTING.md#one-manifest-per-concern, once both surfaces read it) keeping them equal is the contract.
    fn default() -> Self {
        Self {
            preset: "edm".into(),
            params: Params {
                pace: 0.5,
                density: 0.5,
                brightness: 0.5,
                space: 0.5,
                pulse: 0.4,
                texture: 0.4,
            },
            layers: Layers {
                drums: 1.0,
                bass: 1.0,
                pads: 1.0,
                melody: 1.0,
                texture: 1.0,
            },
            seed: 0,
            playing: false,
            volume: 0.65,
            energy: 0.7,
            session: None,
        }
    }
}

fn deserialize_session_patch<'de, D>(deserializer: D) -> Result<Option<Option<Session>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Session>::deserialize(deserializer).map(Some)
}

/// Every field optional: a surface sends what it changed, not the whole state.
/// A full-state PUT would let a stale client silently undo another one's edit.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    preset: Option<String>,
    params: Option<Params>,
    layers: Option<Layers>,
    seed: Option<u32>,
    playing: Option<bool>,
    volume: Option<f32>,
    energy: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_session_patch")]
    session: Option<Option<Session>>,
    /// Who sent this. Echoed back on the stream so the sender can drop its own
    /// change instead of applying a value it already has — without it a dragged
    /// slider fights the round trip of every frame it just sent.
    origin: Option<String>,
}

/// The client currently making sound. Not part of `Scape` on purpose: the scape is
/// what should be playing and is worth persisting, a host is who is doing it right
/// now and is worth nothing after a restart.
#[derive(Clone, Debug, PartialEq, Serialize)]
struct Host {
    id: String,
    /// Free text from the client — "dashboard", "panel", "phone". For humans
    /// reading "playing where", never for routing.
    label: String,
    since_ms: u64,
}

/// A host that has not checked in for this long is gone. Long enough to survive a
/// backgrounded tab's throttled timers, short enough that a closed laptop stops
/// claiming to play before you have walked away from it.
const HOST_TTL: std::time::Duration = std::time::Duration::from_secs(15);

struct Holder {
    host: Host,
    last_seen: std::time::Instant,
}

/// What every surface reads: the scape, plus whether anyone is actually sounding it.
/// `playing` alone answers "what should happen"; `host` answers "is it happening".
#[derive(Clone, Debug, Serialize)]
struct StateView {
    #[serde(flatten)]
    scape: Scape,
    host: Option<Host>,
}

/// A state as it goes out on the stream: the view, flattened so a client that
/// ignores `origin` still reads a plain state, plus who caused it.
#[derive(Clone, Debug, Serialize)]
struct Change {
    #[serde(flatten)]
    view: StateView,
    origin: Option<String>,
}

/// A client claiming the audio output, or keeping its claim alive.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    id: String,
    #[serde(default)]
    label: String,
    /// Take the output from a live host. Never implicit: two tabs both playing is
    /// the defect #113 fixed, and silently stealing output is the other half of it.
    #[serde(default)]
    takeover: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Release {
    id: String,
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

impl Params {
    fn clamped(self) -> Self {
        Self {
            pace: clamp01(self.pace),
            density: clamp01(self.density),
            brightness: clamp01(self.brightness),
            space: clamp01(self.space),
            pulse: clamp01(self.pulse),
            texture: clamp01(self.texture),
        }
    }
}

impl Layers {
    fn clamped(self) -> Self {
        Self {
            drums: clamp01(self.drums),
            bass: clamp01(self.bass),
            pads: clamp01(self.pads),
            melody: clamp01(self.melody),
            texture: clamp01(self.texture),
        }
    }
}

/// Decrements the surface count when a stream ends, however it ends — dropped
/// connection, closed tab, or shutdown. `receiver_count()` cannot answer this since
/// the persister subscribes to the same broadcast and is not a surface.
struct StreamGuard(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct App {
    scape: Arc<RwLock<Scape>>,
    holder: Arc<RwLock<Option<Holder>>>,
    /// Open SSE streams, which is to say surfaces currently watching.
    surfaces: Arc<std::sync::atomic::AtomicUsize>,
    /// Broadcast rather than a client list: a slow reader lags and gets dropped
    /// instead of stalling the writer, which is the right failure for a state feed
    /// where only the newest value matters.
    changes: broadcast::Sender<Change>,
}

impl App {
    /// The four accessors below, and nothing else, take these two locks.
    ///
    /// **A poisoned lock is recovered, not fatal**, and the choice is the same for both
    /// because both hold the same kind of thing: a plain in-memory value with no durable
    /// invariant behind it. `Scape` is what is playing; `Option<Holder>` is which browser
    /// owns the audio output and when it last checked in. Safe Rust cannot leave either half
    /// written, the persister works off a broadcast copy rather than off the lock, and the
    /// worst state a panic can leave is a holder whose `last_seen` is stale — which
    /// [`App::host`] already treats as no holder, because expiry is decided on read.
    ///
    /// `expect("host lock poisoned")` was the shape before, nine times over. One panic under
    /// any guard would have made `GET /api/soundscape/state`, `/stream`, `/host/claim` and
    /// `/host/release` all panic for the life of the process — a panel that is dead until a
    /// restart, with the audio still playing and nothing able to release it. That is a
    /// strictly worse outcome than reading a value the panic did not corrupt.
    ///
    /// The first panic still unwinds and still reaches the runner's stderr. This hides
    /// nothing; it declines to multiply.
    fn scape_read(&self) -> std::sync::RwLockReadGuard<'_, Scape> {
        self.scape
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn scape_write(&self) -> std::sync::RwLockWriteGuard<'_, Scape> {
        self.scape
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn holder_read(&self) -> std::sync::RwLockReadGuard<'_, Option<Holder>> {
        self.holder
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn holder_write(&self) -> std::sync::RwLockWriteGuard<'_, Option<Holder>> {
        self.holder
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The live host, or none. Expiry is decided on read rather than by a clock:
    /// a host is live because it checked in recently, not because a timer has not
    /// fired yet.
    fn host(&self) -> Option<Host> {
        let holder = self.holder_read();
        holder
            .as_ref()
            .filter(|h| h.last_seen.elapsed() < HOST_TTL)
            .map(|h| h.host.clone())
    }

    fn view(&self) -> StateView {
        StateView {
            scape: self.scape_read().clone(),
            host: self.host(),
        }
    }

    fn announce(&self, origin: Option<String>) {
        // Errors mean nobody is listening, which is the normal case with no UI open.
        let _ = self.changes.send(Change {
            view: self.view(),
            origin,
        });
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// What this capability answers, served as data beside `/health`.
/// Required query parameters are named in the summary: a path alone cannot tell
/// a caller what it must send, and learning that from a 400 is the thing this
/// endpoint exists to avoid.
const ROUTES: &[route_manifest::Route] = &[
    r("GET", "/health", "Liveness."),
    r("GET", "/routes", "This manifest."),
    r(
        "GET",
        "/api/soundscape/health",
        "Liveness under the panel prefix. Same handler as /health.",
    ),
    r("GET", "/api/soundscape/state", "Current playback state."),
    r("GET", "/api/soundscape/stream", "The audio stream."),
    r(
        "POST",
        "/api/soundscape/host/claim",
        "Claim playback host for this browser.",
    ),
    r(
        "POST",
        "/api/soundscape/host/release",
        "Release the playback host.",
    ),
];

/// Shorthand so the table above reads as a table.
const fn r(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
) -> route_manifest::Route {
    route_manifest::get(method, path, summary)
}

async fn routes() -> axum::Json<serde_json::Value> {
    axum::Json(route_manifest::manifest("soundscape", ROUTES))
}

async fn get_state(State(app): State<App>) -> Json<StateView> {
    Json(app.view())
}

/// Claim the audio output, or keep an existing claim alive. The same call does
/// both: a heartbeat is just a claim you already hold.
async fn claim_host(
    State(app): State<App>,
    Json(claim): Json<Claim>,
) -> Result<Json<StateView>, (StatusCode, Json<serde_json::Value>)> {
    {
        let mut holder = app.holder_write();
        let live = holder
            .as_ref()
            .filter(|h| h.last_seen.elapsed() < HOST_TTL)
            .map(|h| h.host.clone());

        match live {
            // Someone else is sounding this, and the caller did not say to take it.
            Some(current) if current.id != claim.id && !claim.takeover => {
                return Err((
                    StatusCode::CONFLICT,
                    Json(json!({
                        "error": "another client holds the audio output",
                        "host": current,
                    })),
                ));
            }
            // Our own claim: refresh the heartbeat, keep the original start time so
            // "playing since" does not reset every few seconds.
            Some(current) if current.id == claim.id => {
                *holder = Some(Holder {
                    host: current,
                    last_seen: std::time::Instant::now(),
                });
            }
            _ => {
                *holder = Some(Holder {
                    host: Host {
                        id: claim.id.clone(),
                        label: claim.label.clone(),
                        since_ms: now_ms(),
                    },
                    last_seen: std::time::Instant::now(),
                });
            }
        }
    }

    // The displaced host learns it lost the output the same way every surface
    // learns anything: off the stream. No origin — this concerns everyone.
    app.announce(None);
    Ok(Json(app.view()))
}

/// Give up the output. Only the holder can, so a stale tab cannot silence the one
/// that took over from it.
async fn release_host(State(app): State<App>, Json(release): Json<Release>) -> Json<StateView> {
    let released = {
        let mut holder = app.holder_write();
        match holder.as_ref() {
            Some(h) if h.host.id == release.id => {
                *holder = None;
                true
            }
            _ => false,
        }
    };
    if released {
        app.announce(None);
    }
    Json(app.view())
}

/// A host that stops checking in leaves silence behind, and every surface has to
/// hear about it. Lazy expiry alone would leave "playing" on screen until the next
/// unrelated change happened to arrive.
fn spawn_host_reaper(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(HOST_TTL / 3);
        loop {
            ticker.tick().await;
            let expired = {
                let mut holder = app.holder_write();
                match holder.as_ref() {
                    Some(h) if h.last_seen.elapsed() >= HOST_TTL => {
                        *holder = None;
                        true
                    }
                    _ => false,
                }
            };
            if expired {
                app.announce(None);
            }
        }
    });
}

fn spawn_session_reaper(app: &App) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            ticker.tick().await;
            let finished = {
                let mut scape = app.scape_write();
                let finished = scape
                    .session
                    .as_ref()
                    .is_some_and(|session| session.remaining_at(now_ms()) == 0);
                if finished {
                    scape.session = None;
                    scape.playing = false;
                }
                finished
            };
            if finished {
                app.announce(None);
            }
        }
    });
}

async fn post_state(
    State(app): State<App>,
    Json(patch): Json<Patch>,
) -> Result<Json<StateView>, (StatusCode, Json<serde_json::Value>)> {
    if let Some(preset) = &patch.preset {
        if !PRESETS.contains(&preset.as_str()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("unknown preset {preset:?}"),
                    "known": PRESETS,
                })),
            ));
        }
    }
    if let Some(Some(session)) = &patch.session {
        if !session.is_valid(now_ms()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "invalid soundscape session",
                    "known_scenarios": SCENARIOS,
                    "max_duration_ms": MAX_SESSION_MS,
                })),
            ));
        }
    }

    let origin = patch.origin;
    {
        let mut scape = app.scape_write();
        if let Some(preset) = patch.preset {
            scape.preset = preset;
        }
        if let Some(params) = patch.params {
            scape.params = params.clamped();
        }
        if let Some(layers) = patch.layers {
            scape.layers = layers.clamped();
        }
        if let Some(seed) = patch.seed {
            scape.seed = seed;
        }
        if let Some(playing) = patch.playing {
            scape.playing = playing;
        }
        if let Some(volume) = patch.volume {
            scape.volume = clamp01(volume);
        }
        if let Some(energy) = patch.energy {
            scape.energy = clamp01(energy);
        }
        if let Some(session) = patch.session {
            scape.session = session;
        }
    }

    app.announce(origin);
    Ok(Json(app.view()))
}

/// The current state first, then every change. A client that connects mid-session
/// must not have to also GET /state to know where it is.
async fn stream(
    State(app): State<App>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    // No origin: the opening frame is the state as it stands, not anyone's edit,
    // so every client applies it including the one that last changed it.
    let initial = tokio_stream::once(Change {
        view: app.view(),
        origin: None,
    });
    let updates = BroadcastStream::new(app.changes.subscribe()).filter_map(Result::ok);

    app.surfaces
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let guard = StreamGuard(app.surfaces.clone());

    let events = initial.chain(updates).map(move |change| {
        // Owned by the stream so it lives exactly as long as this connection.
        let _guard = &guard;
        Ok(Event::default()
            .event("state")
            .data(serde_json::to_string(&change).unwrap_or_else(|_| "{}".into())))
    });

    Sse::new(events).keep_alive(axum::response::sse::KeepAlive::default())
}

async fn health(State(app): State<App>) -> Json<serde_json::Value> {
    let host = app.host();
    let scape = app.scape_read();
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "playing": scape.playing,
        // What is actually audible: a wish with no host behind it makes no sound.
        "sounding": scape.playing && host.is_some(),
        "host": host,
        "preset": scape.preset,
        "surfaces": app.surfaces.load(std::sync::atomic::Ordering::Relaxed),
    }))
}

/// `<overlay>/data/soundscape/scape.json`. `None` without an overlay, and then the
/// state is simply in-memory — guessing a location for someone's data is worse than
/// not persisting it.
fn state_path() -> Option<std::path::PathBuf> {
    sjel_config::overlay_data_dir("soundscape").map(|d| d.join("scape.json"))
}

/// What was playing last time, or the defaults. A file we cannot read is reported
/// and stepped over: losing a scape is a nuisance, refusing to start is an outage.
fn load_scape() -> Scape {
    let Some(path) = state_path() else {
        return Scape::default();
    };
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Scape::default();
    };
    match restore(&body) {
        Ok(scape) => scape,
        Err(err) => {
            eprintln!(
                "[soundscape] ignoring unreadable state at {}: {err}",
                path.display()
            );
            Scape::default()
        }
    }
}

fn restore(body: &str) -> Result<Scape, serde_json::Error> {
    let mut scape: Scape = serde_json::from_str(body)?;
    // Never restored: nothing is playing at boot, whatever the file says. A
    // browser that survived the restart re-asserts it on reconnect, and until then
    // `playing: true` here would be a claim with no sound behind it.
    scape.playing = false;
    let now = now_ms();
    if let Some(session) = &mut scape.session {
        session.pause_at(now);
        if !session.is_valid(now) || session.remaining_at(now) == 0 {
            scape.session = None;
        }
    }
    Ok(scape)
}

/// Write through a temp file: a half-written scape.json is worse than an old one,
/// and a rename is the only way to make the swap atomic.
fn save_scape(path: &std::path::Path, scape: &Scape) {
    let Some(dir) = path.parent() else { return };
    if let Err(err) = std::fs::create_dir_all(dir) {
        eprintln!("[soundscape] cannot create {}: {err}", dir.display());
        return;
    }
    let tmp = path.with_extension("json.tmp");
    let body = match serde_json::to_string_pretty(scape) {
        Ok(body) => body,
        Err(err) => {
            eprintln!("[soundscape] cannot serialize state: {err}");
            return;
        }
    };
    if let Err(err) = std::fs::write(&tmp, body) {
        eprintln!("[soundscape] cannot write {}: {err}", tmp.display());
        return;
    }
    if let Err(err) = std::fs::rename(&tmp, path) {
        eprintln!("[soundscape] cannot replace {}: {err}", path.display());
    }
}

/// How long changes pile up before they reach the disk. A dragged slider is one
/// gesture, not forty writes, and the only cost of coalescing is that a crash
/// loses the last couple of seconds of knob-turning.
const SAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);

/// Follows the same broadcast every UI follows, so persistence needs no hook in the
/// write path and cannot fall out of step with what surfaces were told.
fn spawn_persister(app: &App, path: std::path::PathBuf) {
    let mut changes = app.changes.subscribe();
    tokio::spawn(async move {
        loop {
            let Ok(change) = changes.recv().await else {
                // Lagged or closed: the next change carries the whole state, so
                // there is nothing to catch up on.
                continue;
            };
            let mut latest = change.view.scape;
            // Absorb everything that lands inside the window, keeping the newest.
            let deadline = tokio::time::Instant::now() + SAVE_DEBOUNCE;
            loop {
                match tokio::time::timeout_at(deadline, changes.recv()).await {
                    Ok(Ok(next)) => latest = next.view.scape,
                    Ok(Err(_)) => continue,
                    Err(_) => break,
                }
            }
            save_scape(&path, &latest);
        }
    });
}

/// Where the built UI lives. Defaults to what `bun run build` writes (service.toml's
/// `panel_build`), never a checked-in directory; the runner overrides it when the
/// capability is installed somewhere else.
fn ui_dir() -> String {
    sjel_config::env_var("SJEL_SOUNDSCAPE_UI")
        .unwrap_or_else(|_| "capabilities/soundscape/ui/dist".to_string())
}

#[tokio::main]
async fn main() {
    let app = App {
        scape: Arc::new(RwLock::new(load_scape())),
        holder: Arc::new(RwLock::new(None)),
        surfaces: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        changes: broadcast::channel(16).0,
    };
    spawn_host_reaper(&app);
    spawn_session_reaper(&app);

    match state_path() {
        Some(path) => spawn_persister(&app, path),
        None => eprintln!(
            "[soundscape] no SJEL_PERSONAL_ROOT: state is in-memory and will not survive a restart"
        ),
    }

    let dir = ui_dir();
    let index = format!("{dir}/index.html");

    let router = Router::new()
        .route("/routes", get(routes))
        .route("/health", get(health))
        .route("/api/soundscape/health", get(health))
        .route("/api/soundscape/state", get(get_state).post(post_state))
        .route("/api/soundscape/stream", get(stream))
        .route(
            "/api/soundscape/host/claim",
            axum::routing::post(claim_host),
        )
        .route(
            "/api/soundscape/host/release",
            axum::routing::post(release_host),
        )
        .with_state(app)
        // SPA fallback: the bundle has one entry point and routes client-side, so
        // an unknown path is a route, not a 404.
        .fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(&index)));

    let port = sjel_config::resolve_port(Some("SJEL_SOUNDSCAPE_PORT"), None, 8088);
    sjel_server::serve_local("soundscape", port, router).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_out_of_range_params() {
        let wild = Params {
            pace: 2.0,
            density: -1.0,
            brightness: 0.5,
            space: 0.5,
            pulse: 0.5,
            texture: 0.5,
        };
        let safe = wild.clamped();
        assert_eq!(safe.pace, 1.0);
        assert_eq!(safe.density, 0.0);
        assert_eq!(safe.brightness, 0.5);
    }

    #[test]
    fn every_browser_preset_is_known_here() {
        // The browser offers exactly these; a preset it can select but this
        // process rejects would be a 400 the operator cannot explain.
        for preset in ["edm", "ambient", "lofi", "focus", "relax", "sleep"] {
            assert!(PRESETS.contains(&preset), "{preset} missing from PRESETS");
        }
    }

    #[test]
    fn a_patch_leaves_untouched_fields_alone() {
        let patch: Patch = serde_json::from_str(r#"{"preset":"focus"}"#).expect("valid patch");
        assert_eq!(patch.preset.as_deref(), Some("focus"));
        assert!(patch.params.is_none());
        assert!(patch.volume.is_none());
        assert!(patch.session.is_none());
    }

    #[test]
    fn a_session_can_be_cleared_without_looking_like_an_omitted_field() {
        let patch: Patch = serde_json::from_str(r#"{"session":null}"#).expect("valid patch");
        assert_eq!(patch.session, Some(None));
    }

    fn test_app() -> App {
        App {
            scape: Arc::new(RwLock::new(Scape::default())),
            holder: Arc::new(RwLock::new(None)),
            surfaces: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            changes: broadcast::channel(16).0,
        }
    }

    fn claim_of(id: &str, takeover: bool) -> Claim {
        Claim {
            id: id.into(),
            label: id.into(),
            takeover,
        }
    }

    #[tokio::test]
    async fn a_second_client_cannot_quietly_take_the_output() {
        let app = test_app();
        let Json(view) = claim_host(State(app.clone()), Json(claim_of("panel", false)))
            .await
            .expect("first claim wins");
        assert_eq!(
            view.host.expect("claim is reflected in the state").id,
            "panel"
        );

        let refused = claim_host(State(app.clone()), Json(claim_of("dashboard", false))).await;
        let (status, _) = refused.expect_err("second claim is refused");
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(app.host().expect("still held").id, "panel");
    }

    #[tokio::test]
    async fn taking_over_is_allowed_when_it_is_asked_for() {
        let app = test_app();
        let _ = claim_host(State(app.clone()), Json(claim_of("panel", false)))
            .await
            .expect("first claim wins");
        let Json(view) = claim_host(State(app.clone()), Json(claim_of("dashboard", true)))
            .await
            .expect("takeover is honoured");
        assert_eq!(view.host.expect("held").id, "dashboard");
    }

    #[tokio::test]
    async fn a_host_that_stops_checking_in_stops_counting() {
        let app = test_app();
        let _ = claim_host(State(app.clone()), Json(claim_of("panel", false)))
            .await
            .expect("claim wins");

        // Age the heartbeat past the TTL rather than waiting it out.
        {
            let mut holder = app.holder_write();
            let held = holder.as_mut().expect("held");
            held.last_seen = std::time::Instant::now()
                .checked_sub(HOST_TTL + std::time::Duration::from_secs(1))
                .expect("clock has enough history");
        }

        assert!(
            app.host().is_none(),
            "a silent tab does not hold the output"
        );
        // And the seat is free without anyone having to release it.
        let Json(view) = claim_host(State(app.clone()), Json(claim_of("dashboard", false)))
            .await
            .expect("expired host does not block the next one");
        assert_eq!(view.host.expect("held").id, "dashboard");
    }

    #[tokio::test]
    async fn only_the_holder_can_release_the_output() {
        let app = test_app();
        let _ = claim_host(State(app.clone()), Json(claim_of("panel", false)))
            .await
            .expect("claim wins");

        let Json(view) = release_host(
            State(app.clone()),
            Json(Release {
                id: "dashboard".into(),
            }),
        )
        .await;
        assert_eq!(
            view.host.expect("still held").id,
            "panel",
            "a stale client must not silence the one that took over"
        );

        let Json(view) =
            release_host(State(app.clone()), Json(Release { id: "panel".into() })).await;
        assert!(view.host.is_none());
    }

    #[test]
    fn a_restored_scape_never_claims_to_be_playing() {
        let saved = Scape {
            playing: true,
            preset: "sleep".into(),
            ..Scape::default()
        };
        let restored =
            restore(&serde_json::to_string(&saved).expect("serializes")).expect("restores");
        assert!(!restored.playing, "restart cannot resume sound, only state");
        assert_eq!(restored.preset, "sleep", "everything else survives");
    }

    #[test]
    fn an_older_saved_scape_without_a_session_still_restores() {
        let mut value = serde_json::to_value(Scape::default()).expect("serializes");
        value.as_object_mut().expect("object").remove("session");
        let restored =
            restore(&serde_json::to_string(&value).expect("serializes")).expect("restores");
        assert!(restored.session.is_none());
    }

    #[test]
    fn a_running_session_is_paused_across_a_restart() {
        let before = now_ms();
        let saved = Scape {
            playing: true,
            session: Some(Session {
                scenario: "deep-work".into(),
                duration_ms: 60_000,
                elapsed_ms: 5_000,
                running_since_ms: Some(before.saturating_sub(10_000)),
            }),
            ..Scape::default()
        };
        let restored =
            restore(&serde_json::to_string(&saved).expect("serializes")).expect("restores");
        let session = restored.session.expect("session survives");
        assert!(session.running_since_ms.is_none());
        assert!((14_000..=16_000).contains(&session.elapsed_ms));
        assert!(!restored.playing);
    }

    #[tokio::test]
    async fn an_unknown_session_is_rejected() {
        let app = test_app();
        let patch: Patch = serde_json::from_value(json!({
            "session": {
                "scenario": "miracle-cure",
                "duration_ms": 600_000,
                "elapsed_ms": 0,
                "running_since_ms": null
            }
        }))
        .expect("patch shape");
        let result = post_state(State(app), Json(patch)).await;
        let (status, _) = result.expect_err("unknown session rejected");
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_scape_survives_a_write_and_read() {
        let dir = std::env::temp_dir().join(format!("soundscape-test-{}", std::process::id()));
        let path = dir.join("scape.json");
        let saved = Scape {
            preset: "lofi".into(),
            volume: 0.31,
            seed: 4242,
            ..Scape::default()
        };
        save_scape(&path, &saved);

        let body = std::fs::read_to_string(&path).expect("written");
        let restored = restore(&body).expect("restores");
        assert_eq!(restored.preset, "lofi");
        assert_eq!(restored.volume, 0.31);
        assert_eq!(restored.seed, 4242);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_corrupt_state_file_is_not_a_restore() {
        assert!(restore("{ this is not json").is_err());
    }

    #[test]
    fn unknown_fields_are_rejected_rather_than_ignored() {
        // A typo'd field name silently doing nothing is the worst failure mode for
        // a state API driven by two different clients.
        let result: Result<Patch, _> = serde_json::from_str(r#"{"prest":"focus"}"#);
        assert!(result.is_err());
    }

    /// One panic under either lock used to end this capability: nine
    /// `expect("… lock poisoned")` sites meant `GET /state`, `/stream`,
    /// `/host/claim` and `/host/release` all panicked from then on, leaving the
    /// audio playing with nothing able to release it.
    ///
    /// Each lock is poisoned separately, because recovering one and forgetting
    /// the other would still leave the panel dead — and this test is the only
    /// thing that would notice.
    #[tokio::test]
    async fn a_poisoned_lock_leaves_the_panel_answering() {
        for poison_the_holder in [false, true] {
            let app = test_app();
            let victim = app.clone();
            let poisoning = std::thread::spawn(move || {
                if poison_the_holder {
                    let _guard = victim.holder_write();
                    panic!("something panicked while holding the host lock");
                } else {
                    let _guard = victim.scape_write();
                    panic!("something panicked while holding the state lock");
                }
            })
            .join();
            assert!(poisoning.is_err(), "the helper thread must actually panic");

            // Read: what GET /api/soundscape/state and the SSE stream both call.
            assert_eq!(app.view().scape.preset, "edm");

            // Write: the claim path, which takes the host lock and then reads both.
            let Json(view) = claim_host(State(app.clone()), Json(claim_of("panel", false)))
                .await
                .expect("a recovered lock must still admit a claim");
            assert_eq!(view.host.expect("held").id, "panel");
        }
    }
}

#[cfg(test)]
mod route_manifest_tests {
    /// A stale manifest is worse than none, because it gets believed. This reads
    /// the router's own source, so adding a `.route()` without a summary fails
    /// here rather than shipping a surface that lies about itself.
    #[test]
    fn the_manifest_covers_every_served_route() {
        let missing = route_manifest::undeclared_routes(include_str!("main.rs"), super::ROUTES);
        assert!(missing.is_empty(), "served but undocumented: {missing:?}");
    }
}
