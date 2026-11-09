//! On-demand directory scans. The bounded cache lasts only for this status process;
//! neither startup nor GET runs a tool, and no explorer state is written to the database.

use std::collections::VecDeque;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::{extract::Query, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use status_api::{StorageDirectorySnapshot, StorageExplorer};
use tokio::process::Command;

const CACHE_ROOTS: usize = 32;
const TOP: usize = 80;
const SCAN_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default, Deserialize)]
pub(crate) struct RootQuery {
    root: Option<String>,
}

type Refusal = (StatusCode, Json<Value>);

fn refuse(status: StatusCode, error: impl Into<String>) -> Refusal {
    (status, Json(json!({ "error": error.into() })))
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    active: Option<String>,
}

struct Entry {
    view: StorageExplorer,
    complete: Option<(u64, u64)>,
}

fn valid_root(root: &str) -> bool {
    root == "/"
        || (root.starts_with('/')
            && !root.chars().any(char::is_control)
            && root[1..]
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")))
}

fn empty_view(root: &str) -> StorageExplorer {
    StorageExplorer {
        root: root.into(),
        ..StorageExplorer::default()
    }
}

impl Cache {
    fn admitted(&self, root: &str) -> bool {
        valid_root(root)
            && (root == "/"
                || self.entries.iter().any(|entry| {
                    // Cached descendants retain parent navigation after an ancestor is evicted.
                    Path::new(&entry.view.root).starts_with(root)
                        || entry.view.snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot.directories.iter().any(|child| child.path == root)
                        })
                }))
    }

    fn get(&self, root: &str) -> Result<StorageExplorer, Refusal> {
        if !self.admitted(root) {
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                "root must be / or a directory listed in a previous scan",
            ));
        }
        Ok(self
            .entries
            .iter()
            .find(|entry| entry.view.root == root)
            .map(|entry| entry.view.clone())
            .unwrap_or_else(|| empty_view(root)))
    }

    fn begin(&mut self, root: &str) -> Result<(StorageExplorer, bool), Refusal> {
        self.get(root)?;
        if let Some(active) = &self.active {
            if active == root {
                return self.get(root).map(|view| (view, false));
            }
            return Err(refuse(
                StatusCode::CONFLICT,
                format!("a storage scan is already running for {active}; retry after it completes"),
            ));
        }
        let mut entry = self
            .entries
            .iter()
            .position(|entry| entry.view.root == root)
            .and_then(|index| self.entries.remove(index))
            .unwrap_or_else(|| Entry {
                view: empty_view(root),
                complete: None,
            });
        entry.view.scanning = true;
        entry.view.error = None;
        let view = entry.view.clone();
        if self.entries.len() == CACHE_ROOTS {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
        self.active = Some(root.into());
        Ok((view, true))
    }

    fn finish(&mut self, root: &str, result: Result<StorageDirectorySnapshot, String>) {
        if self.active.as_deref() != Some(root) {
            return;
        }
        self.active = None;
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.view.root == root)
        else {
            return;
        };
        entry.view.scanning = false;
        match result.and_then(|snapshot| validate_snapshot(root, snapshot)) {
            Ok(snapshot) => {
                entry.view.error = None;
                if !snapshot.partial && snapshot.error.is_none() {
                    (entry.view.previous_bytes, entry.view.previous_measured_at) =
                        match entry.complete {
                            Some((bytes, at)) => (Some(bytes), Some(at)),
                            None => (None, None),
                        };
                    entry.complete = Some((snapshot.bytes, snapshot.measured_at));
                } else {
                    // Partial/error readings are visible but cannot become growth baselines.
                    entry.view.previous_bytes = None;
                    entry.view.previous_measured_at = None;
                }
                entry.view.snapshot = Some(snapshot);
            }
            Err(error) => entry.view.error = Some(error),
        }
    }
}

fn validate_snapshot(
    root: &str,
    snapshot: StorageDirectorySnapshot,
) -> Result<StorageDirectorySnapshot, String> {
    if snapshot.root != root
        || snapshot.directories.len() > TOP
        || snapshot.directories.iter().any(|child| {
            !valid_root(&child.path) || Path::new(&child.path).parent() != Some(Path::new(root))
        })
    {
        return Err("sjel-storage returned an invalid directory snapshot".into());
    }
    // An error-bearing reading must never yield a growth value through the shared model.
    let mut snapshot = snapshot;
    snapshot.partial |= snapshot.error.is_some();
    Ok(snapshot)
}

#[derive(Clone, Default)]
struct Explorer(Arc<Mutex<Cache>>);

impl Explorer {
    fn get(&self, root: &str) -> Result<StorageExplorer, Refusal> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get(root)
    }

    fn start<F, Fut>(&self, root: &str, scan: F) -> Result<StorageExplorer, Refusal>
    where
        F: FnOnce(String) -> Fut + Send + 'static,
        Fut: Future<Output = Result<StorageDirectorySnapshot, String>> + Send + 'static,
    {
        let (view, started) = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .begin(root)?;
        if started {
            let cache = self.0.clone();
            let root = root.to_owned();
            tokio::spawn(async move {
                let result = scan(root.clone()).await;
                cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .finish(&root, result);
            });
        }
        Ok(view)
    }
}

fn explorer() -> &'static Explorer {
    static EXPLORER: OnceLock<Explorer> = OnceLock::new();
    EXPLORER.get_or_init(Explorer::default)
}

/// Read cached state without starting filesystem discovery.
///
/// # Errors
/// Refuses roots that have not been discovered or are not canonical paths.
pub(crate) async fn get(Query(query): Query<RootQuery>) -> Result<Json<StorageExplorer>, Refusal> {
    explorer()
        .get(query.root.as_deref().unwrap_or("/"))
        .map(Json)
}

/// Start an explicitly requested scan and return its current state.
///
/// # Errors
/// Refuses unsupported roots or a different root while a scan is running.
pub(crate) async fn post(
    Query(query): Query<RootQuery>,
) -> Result<(StatusCode, Json<StorageExplorer>), Refusal> {
    explorer()
        .start(query.root.as_deref().unwrap_or("/"), scan)
        .map(|view| (StatusCode::ACCEPTED, Json(view)))
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    metadata.is_file()
}

fn storage_binary() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SJEL_STORAGE_BIN") {
        let path = PathBuf::from(path);
        return executable(&path)
            .then_some(path)
            .ok_or_else(|| "SJEL_STORAGE_BIN is not an executable file".into());
    }
    if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
        let target = PathBuf::from(target);
        let target = if target.is_absolute() {
            target
        } else {
            super::sjel_root()?.join(target)
        };
        let path = target.join("release/sjel-storage");
        if executable(&path) {
            return Ok(path);
        }
    }
    let path = super::sjel_root()?.join("target/release/sjel-storage");
    executable(&path).then_some(path).ok_or_else(|| {
        "prebuilt sjel-storage is unavailable; build it explicitly or set SJEL_STORAGE_BIN".into()
    })
}

fn actual_root(root: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let data = Path::new("/System/Volumes/Data");
        if data.is_dir() {
            return data.join(root.trim_start_matches('/'));
        }
    }
    PathBuf::from(root)
}

async fn scan(root: String) -> Result<StorageDirectorySnapshot, String> {
    let binary = storage_binary()?;
    let mut command = Command::new(binary);
    command
        .arg("browse")
        .arg("--root")
        .arg(actual_root(&root))
        .args(["--top", "80", "--json"]);
    let output = run_command(command, SCAN_TIMEOUT).await?;
    decode_output(output)
}

fn decode_output(output: Output) -> Result<StorageDirectorySnapshot, String> {
    if !output.status.success() {
        return Err(format!(
            "sjel-storage failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(4096)
                .collect::<String>()
                .trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("sjel-storage emitted malformed JSON: {error}"))
}

// The tool can have a du child holding its pipes. kill_on_drop alone kills only the
// tool; a separate process group also ends du on timeout or cancellation.
struct ProcessGroup(Option<u32>);

impl ProcessGroup {
    fn terminate(&mut self) -> Result<(), String> {
        let Some(id) = self.0.take() else {
            return Ok(());
        };
        #[cfg(unix)]
        {
            let status = std::process::Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{id}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("could not terminate storage process group: {error}"))?;
            if !status.success() {
                return Err(format!(
                    "storage process-group termination failed ({status})"
                ));
            }
        }
        Ok(())
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let Some(id) = self.0.take() else { return };
        std::thread::spawn(move || {
            if let Err(error) = ProcessGroup(Some(id)).terminate() {
                eprintln!("[sjel-status] {error}");
            }
        });
    }
}

async fn run_command(mut command: Command, timeout: Duration) -> Result<Output, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .spawn()
        .map_err(|error| format!("could not start sjel-storage: {error}"))?;
    let id = child.id();
    let mut output = Box::pin(child.wait_with_output());
    // Drop order matters: kill the group before dropping the future owning the child.
    let mut group = ProcessGroup(id);
    match tokio::time::timeout(timeout, &mut output).await {
        Ok(Ok(output)) => {
            group.0 = None;
            Ok(output)
        }
        Ok(Err(error)) => Err(format!("could not read sjel-storage output: {error}")),
        Err(_) => {
            let mut timed_out_group = ProcessGroup(group.0.take());
            tokio::task::spawn_blocking(move || timed_out_group.terminate())
                .await
                .map_err(|error| format!("storage termination task failed: {error}"))??;
            // Reap the leader and drain the closed pipes after killing the whole group.
            tokio::time::timeout(Duration::from_secs(2), &mut output)
                .await
                .map_err(|_| {
                    "storage process group did not close its pipes after termination".to_string()
                })?
                .map_err(|error| format!("could not reap sjel-storage after timeout: {error}"))?;
            Err(format!(
                "sjel-storage scan timed out after {}s",
                timeout.as_secs()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use status_api::StorageDirectory;

    fn snapshot(root: &str, bytes: u64, at: u64) -> StorageDirectorySnapshot {
        StorageDirectorySnapshot {
            root: root.into(),
            measured_at: at,
            bytes,
            directories: vec![],
            unlisted_bytes: bytes,
            omitted_directories: 0,
            partial: false,
            error: None,
        }
    }

    fn seed(cache: &mut Cache) {
        cache.begin("/").unwrap();
        let mut root = snapshot("/", 100, 1);
        root.directories.push(StorageDirectory {
            path: "/data".into(),
            bytes: 50,
        });
        cache.finish("/", Ok(root));
    }

    #[test]
    fn admission_requires_discovery_and_rejects_noncanonical_paths() {
        let mut cache = Cache::default();
        for root in [
            "", "data", "/data", "/../", "/./data", "/data/..", "//data", "/data/", "/data\n",
        ] {
            assert!(cache.get(root).is_err(), "{root:?}");
        }
        assert!(cache.get("/").is_ok());
        seed(&mut cache);
        assert!(cache.get("/data").is_ok());
        assert!(cache.get("/data/unknown").is_err());
        assert!(cache.get("/database").is_err());
    }

    #[test]
    fn single_flight_is_global_and_does_not_queue_roots() {
        let mut cache = Cache::default();
        seed(&mut cache);
        assert!(cache.begin("/").unwrap().1);
        assert!(!cache.begin("/").unwrap().1);
        assert_eq!(cache.begin("/data").unwrap_err().0, StatusCode::CONFLICT);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.active.as_deref(), Some("/"));
    }

    #[test]
    fn growth_uses_complete_same_root_snapshots_across_partial_and_failed_refreshes() {
        let mut cache = Cache::default();
        seed(&mut cache);
        assert_eq!(cache.get("/").unwrap().growth_bytes(), None);
        cache.begin("/").unwrap();
        cache.finish("/", Ok(snapshot("/", 150, 2)));
        let complete = cache.get("/").unwrap();
        assert_eq!(complete.growth_bytes(), Some(50));
        assert_eq!(complete.previous_measured_at, Some(1));
        let refreshing = cache.begin("/").unwrap().0;
        assert_eq!(refreshing.snapshot, complete.snapshot);
        cache.finish("/", Err("failed refresh".into()));
        let failed = cache.get("/").unwrap();
        assert_eq!(failed.snapshot, complete.snapshot);
        assert_eq!(failed.previous_bytes, complete.previous_bytes);
        assert_eq!(failed.error.as_deref(), Some("failed refresh"));
        cache.begin("/").unwrap();
        let mut partial = snapshot("/", 40, 3);
        partial.partial = true;
        cache.finish("/", Ok(partial));
        assert_eq!(cache.get("/").unwrap().growth_bytes(), None);
        cache.begin("/").unwrap();
        cache.finish("/", Ok(snapshot("/", 175, 4)));
        let recovered = cache.get("/").unwrap();
        assert_eq!(recovered.growth_bytes(), Some(25));
        assert_eq!(recovered.previous_measured_at, Some(2));
    }

    #[test]
    fn invalid_snapshots_preserve_prior_reading_and_do_not_admit_paths() {
        let mut cache = Cache::default();
        seed(&mut cache);
        let before = cache.get("/").unwrap().snapshot;
        for invalid in [snapshot("/wrong", 1, 2), {
            let mut value = snapshot("/", 1, 2);
            value.directories.push(StorageDirectory {
                path: "/data/../secret".into(),
                bytes: 1,
            });
            value
        }] {
            cache.begin("/").unwrap();
            cache.finish("/", Ok(invalid));
            assert_eq!(cache.get("/").unwrap().snapshot, before);
            assert!(cache.get("/").unwrap().error.is_some());
        }
    }

    #[test]
    fn partial_snapshots_allow_listed_directories_but_do_not_establish_growth() {
        let mut cache = Cache::default();
        cache.begin("/").unwrap();
        let mut partial = snapshot("/", 5, 1);
        partial.error = Some("permission denied".into());
        partial.directories.push(StorageDirectory {
            path: "/secret".into(),
            bytes: 5,
        });
        cache.finish("/", Ok(partial));
        assert!(cache.get("/secret").is_ok());
        assert!(cache.get("/unlisted").is_err());
        let view = cache.get("/").unwrap();
        let reading = view.snapshot.unwrap();
        assert!(reading.partial);
        assert_eq!(reading.error.as_deref(), Some("permission denied"));
        assert!(view.error.is_none());
        cache.begin("/").unwrap();
        cache.finish("/", Ok(snapshot("/", 10, 2)));
        assert_eq!(cache.get("/").unwrap().growth_bytes(), None);
    }

    #[test]
    fn cache_is_bounded_and_cached_ancestors_remain_navigable() {
        let mut cache = Cache::default();
        let mut root = "/".to_string();
        for at in 0..(CACHE_ROOTS + 5) {
            cache.begin(&root).unwrap();
            let next = format!("{}/child", root.trim_end_matches('/'));
            let mut value = snapshot(&root, 1, at as u64);
            value.directories.push(StorageDirectory {
                path: next.clone(),
                bytes: 1,
            });
            cache.finish(&root, Ok(value));
            root = next;
            assert!(cache.entries.len() <= CACHE_ROOTS);
        }
        assert_eq!(cache.entries.len(), CACHE_ROOTS);
        assert!(cache.entries.iter().all(|entry| entry.view.root != "/"));
        assert!(cache.get("/").is_ok());
        assert!(cache.get("/child").is_ok());
        assert!(cache.get("/childish").is_err());
    }

    #[tokio::test]
    async fn get_is_wired_and_never_starts_a_scan() {
        use axum::{
            body::{to_bytes, Body},
            http::Request,
        };
        use tower::ServiceExt;
        let router = super::super::build_router(super::super::proxy::Proxy::new(&[], "8082"));
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/sjel-status/storage-explorer")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
        let state: StorageExplorer = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(state, empty_view("/"));
        assert!(explorer().0.lock().unwrap().entries.is_empty());
    }

    #[tokio::test]
    async fn post_is_wired_and_refuses_undiscovered_roots_without_starting_a_scan() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;
        let router = super::super::build_router(super::super::proxy::Proxy::new(&[], "8082"));
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/sjel-status/storage-explorer?root=%2Funknown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(explorer().0.lock().unwrap().entries.is_empty());
    }

    #[tokio::test]
    async fn explicit_start_returns_scanning_state_before_injected_scan_finishes() {
        let explorer = Explorer::default();
        let (send, receive) = tokio::sync::oneshot::channel();
        let view = explorer
            .start("/", move |_| async move { receive.await.unwrap() })
            .unwrap();
        assert!(view.scanning);
        assert!(view.snapshot.is_none());
        assert!(
            explorer
                .start("/", |_| async { panic!("duplicate scan") })
                .unwrap()
                .scanning
        );
        send.send(Ok(snapshot("/", 5, 1))).unwrap();
        for _ in 0..100 {
            if !explorer.get("/").unwrap().scanning {
                assert_eq!(explorer.get("/").unwrap().snapshot.unwrap().bytes, 5);
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("injected scan did not complete");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_command_failures_and_malformed_json_are_errors() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf denied >&2; exit 7"]);
        let error =
            decode_output(run_command(command, Duration::from_secs(2)).await.unwrap()).unwrap_err();
        assert!(error.contains("7") && error.contains("denied"), "{error}");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf invalid"]);
        let error =
            decode_output(run_command(command, Duration::from_secs(2)).await.unwrap()).unwrap_err();
        assert!(error.contains("malformed JSON"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_terminates_the_tool_and_its_pipe_holding_child() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 60 & wait"]);
        let started = std::time::Instant::now();
        let error = run_command(command, Duration::from_millis(30))
            .await
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
