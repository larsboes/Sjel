//! `GET /api/sjel-status/events`: the health and agent answers, pushed when either changes.
//!
//! A client that follows this stream reads no page on a timer. One watcher serves every
//! connection, probes only while one is open, and sends a frame only when the answer differs
//! from the last one, so a quiet machine costs one cached read every two seconds.

use super::*;

use std::convert::Infallible;
use std::sync::Arc;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use tokio::sync::broadcast;

/// How often a connected client is checked for a change.
const TICK: Duration = Duration::from_secs(2);
/// A comment frame keeps the connection open across a quiet minute.
const KEEP_ALIVE: Duration = Duration::from_secs(50);
/// A client that falls this far behind loses whole frames, never part of one.
const CHANNEL: usize = 8;

/// The stream of snapshots this shell pushes: the current one at once, then one per change.
pub(crate) async fn events_handler() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (_, frame) = snapshot().await;
    let opening = stream::once(async move { Ok(snapshot_event(frame)) });
    let events = stream::unfold(watcher().subscribe(), |mut frames| async move {
        loop {
            match frames.recv().await {
                Ok(frame) => return Some((Ok(snapshot_event(frame.as_str().to_owned())), frames)),
                // A slow reader lost frames. The next one carries every answer.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(opening.chain(events)).keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
}

/// The one watcher, started by the first connection and kept for the process's life.
fn snapshot_event(frame: String) -> Event {
    Event::default().event("snapshot").data(frame)
}

fn watcher() -> &'static broadcast::Sender<Arc<String>> {
    static WATCHER: OnceLock<broadcast::Sender<Arc<String>>> = OnceLock::new();
    WATCHER.get_or_init(|| {
        let (send, _) = broadcast::channel(CHANNEL);
        let watched = send.clone();
        tokio::spawn(async move { watch(watched).await });
        send
    })
}

async fn watch(send: broadcast::Sender<Arc<String>>) {
    let mut last: Option<Value> = None;
    loop {
        if send.receiver_count() == 0 {
            // A frame sent with nobody listening is not the last answer anyone saw. The loop
            // still wakes, so a new connection's first change is at most one tick away; only
            // the probe is skipped.
            last = None;
            tokio::time::sleep(TICK).await;
            continue;
        }
        let (fingerprint, frame) = snapshot().await;
        if last.as_ref() != Some(&fingerprint) {
            last = Some(fingerprint);
            let _ = send.send(Arc::new(frame));
        }
        tokio::time::sleep(TICK).await;
    }
}

/// Both answers as one JSON object, or the reason one could not be read, with the value that
/// says whether anything changed.
///
/// The fingerprint leaves out the uptime, which moves every second; a reader adds its own
/// elapsed time to the uptime it was sent (`Workspace::sidebar` in `apps/gpui`). Without that,
/// every frame would differ and the watcher would send one per tick. It is compared as a value,
/// not as text: the capabilities come out of a map, and their key order is not the same twice.
async fn snapshot() -> (Value, String) {
    let mut frame = serde_json::Map::new();
    let mut fingerprint = serde_json::Map::new();
    match sjel_status_health_handler().await {
        Ok(Json(health)) => {
            let health = serde_json::to_value(health).unwrap_or(Value::Null);
            let mut steady = health.clone();
            if let Some(steady) = steady.as_object_mut() {
                steady.insert("uptime_seconds".into(), Value::from(0));
            }
            fingerprint.insert("health".into(), steady);
            frame.insert("health".into(), health);
        }
        Err((_, Json(error))) => {
            fingerprint.insert("error".into(), error.clone());
            frame.insert("error".into(), error);
        }
    }
    match agent_handler().await {
        Ok(Json(agent)) => {
            fingerprint.insert("agent".into(), agent.clone());
            frame.insert("agent".into(), agent);
        }
        Err((_, Json(error))) => {
            fingerprint.insert("error".into(), error.clone());
            frame.insert("error".into(), error);
        }
    }
    (Value::Object(fingerprint), Value::Object(frame).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_snapshot_carries_both_answers_as_one_object() {
        let (_, frame) = snapshot().await;
        let frame: Value = serde_json::from_str(&frame).unwrap();
        assert!(frame.get("health").is_some() || frame.get("error").is_some());
        assert!(frame.get("agent").is_some() || frame.get("error").is_some());
    }

    #[tokio::test]
    async fn the_uptime_alone_does_not_count_as_a_change() {
        let (first, frame) = snapshot().await;
        let frame: Value = serde_json::from_str(&frame).unwrap();
        assert!(frame.get("health").is_some() || frame.get("error").is_some());

        let (second, _) = snapshot().await;
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn a_frame_carries_the_uptime_the_fingerprint_leaves_out() {
        let (fingerprint, frame) = snapshot().await;
        let frame: Value = serde_json::from_str(&frame).unwrap();
        if let Some(uptime) = frame.pointer("/health/uptime_seconds") {
            assert_eq!(
                fingerprint.pointer("/health/uptime_seconds"),
                Some(&Value::from(0))
            );
            assert!(uptime.is_u64());
        }
    }
}
