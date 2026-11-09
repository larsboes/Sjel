//! Cached directory allocation discovery. Only an explicit scan action starts a disk walk.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, IntoElement};

use crate::data::Remote;
use crate::primitives::{list, mono_num, note, row};
use crate::theme::Palette;
use sjel_gpui_core::shell::{self, ShellError, ShellResult};
use sjel_gpui_core::systems::{storage_bytes, StorageExplorer};

use super::{heading, SystemsPage};

const MAX_ROWS: usize = 80;
const MAX_CACHED_ROOTS: usize = 32;
const POLL_BEAT: Duration = Duration::from_secs(2);
const POLL_BUDGET: Duration = Duration::from_secs(180);
const MAX_POLLS: usize = 90;

pub(super) struct ExplorerState {
    root: String,
    remote: Remote<StorageExplorer>,
    cache: HashMap<String, StorageExplorer>,
    request: u64,
    following: bool,
    timed_out: bool,
}

impl Default for ExplorerState {
    fn default() -> Self {
        Self {
            root: "/".into(),
            remote: Remote::default(),
            cache: HashMap::new(),
            request: 0,
            following: false,
            timed_out: false,
        }
    }
}

impl ExplorerState {
    pub(super) fn stop(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.following = false;
        self.remote.loading = false;
    }

    fn navigate(&mut self, root: String) {
        self.stop();
        self.remote = Remote::default();
        if let Some(cached) = self.cache.get(&root) {
            self.remote.settle(Ok(cached.clone()));
        }
        self.root = root;
        self.timed_out = false;
    }

    fn begin(&mut self) -> Option<u64> {
        if self.following || !self.remote.begin() {
            return None;
        }
        self.request = self.request.wrapping_add(1);
        self.following = true;
        self.timed_out = false;
        Some(self.request)
    }

    fn current(&self, request: u64) -> bool {
        self.request == request && self.following
    }

    fn settle(&mut self, request: u64, result: ShellResult<StorageExplorer>) -> bool {
        if !self.current(request) {
            return false;
        }
        let result = result.and_then(|view| {
            if view.root != self.root
                || view
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.root != self.root)
            {
                return Err(ShellError::Unreadable);
            }
            if !self.cache.contains_key(&self.root) && self.cache.len() >= MAX_CACHED_ROOTS {
                if let Some(evicted) = self.cache.keys().next().cloned() {
                    self.cache.remove(&evicted);
                }
            }
            self.cache.insert(self.root.clone(), view.clone());
            Ok(view)
        });
        let scanning = result.as_ref().is_ok_and(|view| view.scanning);
        self.remote.settle(result);
        self.following = scanning;
        scanning
    }

    fn timeout(&mut self) {
        self.stop();
        self.timed_out = true;
    }

    fn scanning(&self) -> bool {
        self.remote.value.as_ref().is_some_and(|view| view.scanning)
    }

    fn action(&self) -> &'static str {
        if self.following {
            "Checking scan…"
        } else if self.scanning() || self.timed_out {
            "Check scan status"
        } else if self
            .remote
            .value
            .as_ref()
            .is_some_and(|view| view.snapshot.is_some())
        {
            "Refresh scan"
        } else if self.root == "/" {
            "Scan disk"
        } else {
            "Scan folder"
        }
    }
}

impl SystemsPage {
    pub(super) fn read_explorer(&mut self, scan: bool, cx: &mut Context<Self>) {
        let Some(request) = self.explorer.begin() else {
            return;
        };
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.explorer.settle(request, Err(error));
                cx.notify();
                return;
            }
        };
        let root = self.explorer.root.clone();
        cx.notify();
        cx.spawn(async move |weak, cx| {
            let started = Instant::now();
            for poll in 0..=MAX_POLLS {
                if poll > 0 {
                    cx.background_executor().timer(POLL_BEAT).await;
                }
                let live = weak
                    .update(cx, |page, cx| {
                        if !page.explorer.current(request) {
                            return false;
                        }
                        if poll > 0 && started.elapsed() >= POLL_BUDGET {
                            page.explorer.timeout();
                            cx.notify();
                            return false;
                        }
                        page.explorer.remote.begin();
                        true
                    })
                    .unwrap_or(false);
                if !live {
                    return;
                }
                let shell = shell.clone();
                let root = root.clone();
                let work = shell::run(async move {
                    if scan && poll == 0 {
                        shell.scan_storage(&root).await
                    } else {
                        shell.storage_explorer(&root).await
                    }
                });
                let result = work.await.unwrap_or(Err(ShellError::Unreachable));
                let running = weak
                    .update(cx, |page, cx| {
                        if !page.explorer.current(request) {
                            return false;
                        }
                        let running = page.explorer.settle(request, result);
                        if running && (poll == MAX_POLLS || started.elapsed() >= POLL_BUDGET) {
                            page.explorer.timeout();
                            cx.notify();
                            return false;
                        }
                        cx.notify();
                        running
                    })
                    .unwrap_or(false);
                if !running {
                    return;
                }
            }
        })
        .detach();
    }

    fn navigate_explorer(&mut self, root: String, cx: &mut Context<Self>) {
        self.explorer.navigate(root);
        self.read_explorer(false, cx);
    }

    pub(super) fn explorer_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.explorer;
        let view = state.remote.value.as_ref();
        let snapshot = view.and_then(|view| view.snapshot.as_ref());
        let parent = parent_root(&state.root);
        let mut body = div().flex().flex_col().gap_2()
            .child(heading("Disk explorer", palette))
            .child(
                div().flex().items_center().gap_2()
                    .children(parent.map(|parent| {
                        Button::new("disk-explorer-back").small().ghost().label("Back")
                            .tooltip(format!("Back to {parent}"))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.navigate_explorer(parent.clone(), cx);
                            }))
                    }))
                    .when(state.root != "/", |bar| bar.child(
                        Button::new("disk-explorer-root").small().ghost().label("Disk root")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.navigate_explorer("/".into(), cx);
                            }))
                    ))
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(state.root.clone()))
                    .child(
                        Button::new("disk-explorer-scan").small().label(state.action())
                            .disabled(state.following)
                            .tooltip(if state.following {
                                "Waiting for this scan; previous measurements remain visible."
                            } else {
                                "Scans run only on request. Allocated bytes are not recoverable-space estimates."
                            })
                            .on_click(cx.listener(|page, _, _, cx| {
                                let scan = !page.explorer.scanning() && !page.explorer.timed_out;
                                page.read_explorer(scan, cx);
                            })),
                    ),
            )
            .child(note("Allocated bytes, not recoverable space. / represents the whole writable volume. No files are changed.", palette.muted));

        if state.following {
            body = body.child(note(
                if state.scanning() {
                    "Scanning… Previous measurements remain visible until the scan finishes."
                } else {
                    "Reading scan state…"
                },
                palette.muted,
            ));
        }
        if state.timed_out {
            body = body.child(note("Stopped waiting after three minutes. The server scan may still be running. Check scan status to resume; no new scan is started.", palette.amber));
        }
        if let Some(error) = &state.remote.error {
            body = body.child(note(format!("Could not read scan state: {error}. Previous measurements retained. Retry with the action above."), palette.red));
        }
        if let Some(error) = view.and_then(|view| view.error.as_ref()) {
            body = body.child(note(
                format!("Scan failed: {error}. Retry with the action above."),
                palette.red,
            ));
        }
        if let Some(snapshot) = snapshot {
            body = body.child(
                row()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Measured {}", measured_label(snapshot.measured_at))),
                    )
                    .child(
                        mono_num(
                            format!("{} allocated", storage_bytes(snapshot.bytes)),
                            palette.text,
                        )
                        .text_sm(),
                    ),
            );
            if let Some(growth) = view.and_then(growth_label) {
                body = body.child(mono_num(growth, palette.muted).text_xs());
            }
            if snapshot.partial {
                body = body.child(note("Partial scan: some paths could not be measured. Totals are incomplete; growth is unavailable. Refresh scan to retry.", palette.amber));
            }
            if let Some(error) = &snapshot.error {
                body = body.child(note(error.clone(), palette.amber));
            }
            let rows = snapshot
                .directories
                .iter()
                .take(MAX_ROWS)
                .enumerate()
                .map(|(index, entry)| {
                    let path = entry.path.clone();
                    row()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(path.clone()),
                        )
                        .child(
                            mono_num(storage_bytes(entry.bytes), palette.text)
                                .w(px(110.))
                                .text_xs(),
                        )
                        .child(
                            Button::new(("disk-explorer-directory", index))
                                .small()
                                .ghost()
                                .label("Open")
                                .tooltip(format!("Open {path}"))
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    page.navigate_explorer(path.clone(), cx);
                                })),
                        )
                        .into_any_element()
                })
                .collect();
            body = body.child(list(rows, palette))
                .child(row()
                    .child(div().flex_1().min_w_0().text_xs().text_color(rgb(palette.muted))
                        .child(format!("Unlisted: direct files or omitted directories · {} directories omitted", snapshot.omitted_directories)))
                    .child(mono_num(storage_bytes(snapshot.unlisted_bytes), palette.muted).w(px(110.)).text_xs()));
            if snapshot.directories.len() > MAX_ROWS {
                body = body.child(note("Showing the first 80 directories.", palette.muted));
            }
            if snapshot.directories.is_empty() {
                body = body.child(note("No child directories in this snapshot. Direct files are included in unlisted bytes.", palette.muted));
            }
        } else {
            body = body.child(note("No cached measurement for this path. Choose Scan disk or Scan folder to measure it. Opening folders never starts a scan.", palette.muted));
        }
        body.into_any_element()
    }
}

fn parent_root(root: &str) -> Option<String> {
    Path::new(root)
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
}

fn measured_label(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .map(|stamp| {
            stamp
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %Z")
                .to_string()
        })
        .unwrap_or_else(|| format!("{seconds} seconds since Unix epoch"))
}

fn growth_label(view: &StorageExplorer) -> Option<String> {
    let delta = view.growth_bytes()?;
    let bytes = u64::try_from(delta.unsigned_abs()).ok()?;
    let sign = if delta > 0 {
        "+"
    } else if delta < 0 {
        "−"
    } else {
        "±"
    };
    let size = if bytes < 1024 * 1024 {
        format!("{bytes} B")
    } else {
        storage_bytes(bytes)
    };
    Some(format!(
        "{sign}{size} since {}",
        measured_label(view.previous_measured_at?)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_gpui_core::systems::StorageDirectorySnapshot;

    fn view(root: &str, bytes: u64, scanning: bool) -> StorageExplorer {
        StorageExplorer {
            root: root.into(),
            snapshot: Some(StorageDirectorySnapshot {
                root: root.into(),
                measured_at: 20,
                bytes,
                directories: vec![],
                unlisted_bytes: bytes,
                omitted_directories: 0,
                partial: false,
                error: None,
            }),
            scanning,
            previous_bytes: Some(100),
            previous_measured_at: Some(10),
            error: None,
        }
    }

    #[test]
    fn navigation_never_displays_another_roots_late_response() {
        let mut state = ExplorerState::default();
        let root_request = state.begin().unwrap();
        state.navigate("/Users".into());
        let child_request = state.begin().unwrap();
        assert!(!state.settle(root_request, Ok(view("/", 400, false))));
        assert!(state.remote.value.is_none());
        state.settle(child_request, Ok(view("/Users", 200, false)));
        assert_eq!(state.remote.value.as_ref().unwrap().root, "/Users");
    }

    #[test]
    fn returning_to_a_root_rejects_its_old_request_and_keeps_its_cached_tuple() {
        let mut state = ExplorerState::default();
        let first = state.begin().unwrap();
        state.settle(first, Ok(view("/", 300, true)));
        state.navigate("/Users".into());
        state.navigate("/".into());
        let latest = state.begin().unwrap();
        assert!(!state.settle(first, Ok(view("/", 900, false))));
        assert_eq!(
            state
                .remote
                .value
                .as_ref()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .bytes,
            300
        );
        state.settle(latest, Ok(view("/", 500, false)));
        assert_eq!(
            state.remote.value.as_ref().unwrap().growth_bytes(),
            Some(400)
        );
    }

    #[test]
    fn mismatched_payload_and_failed_refresh_preserve_the_previous_snapshot() {
        let mut state = ExplorerState::default();
        let first = state.begin().unwrap();
        state.settle(first, Ok(view("/", 300, false)));
        let refresh = state.begin().unwrap();
        state.settle(refresh, Ok(view("/wrong", 500, false)));
        assert_eq!(state.remote.error, Some(ShellError::Unreadable));
        assert_eq!(
            state
                .remote
                .value
                .as_ref()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .bytes,
            300
        );
        let retry = state.begin().unwrap();
        state.settle(retry, Err(ShellError::Unreachable));
        assert_eq!(state.remote.value.as_ref().unwrap().root, "/");
    }

    #[test]
    fn timeout_and_stop_invalidate_inflight_reads_without_losing_data() {
        let mut state = ExplorerState::default();
        assert_eq!(state.action(), "Scan disk");
        let request = state.begin().unwrap();
        assert!(state.begin().is_none());
        state.settle(request, Ok(view("/", 300, true)));
        state.timeout();
        assert_eq!(state.action(), "Check scan status");
        assert!(!state.settle(request, Ok(view("/", 900, false))));
        assert_eq!(
            state
                .remote
                .value
                .as_ref()
                .unwrap()
                .snapshot
                .as_ref()
                .unwrap()
                .bytes,
            300
        );
        state.navigate("/Users".into());
        assert_eq!(state.action(), "Scan folder");
        let next = state.begin().unwrap();
        state.stop();
        assert!(!state.settle(next, Ok(view("/Users", 200, false))));
    }

    #[test]
    fn growth_is_signed_and_never_shown_for_partial_snapshots() {
        let mut complete = view("/", 60, false);
        assert!(growth_label(&complete).unwrap().starts_with("−40 B since "));
        complete.snapshot.as_mut().unwrap().bytes = 140;
        assert!(growth_label(&complete).unwrap().starts_with("+40 B since "));
        complete.snapshot.as_mut().unwrap().partial = true;
        assert!(growth_label(&complete).is_none());
    }

    #[test]
    fn local_snapshot_cache_is_bounded() {
        let mut state = ExplorerState::default();
        for index in 0..(MAX_CACHED_ROOTS + 2) {
            let root = format!("/folder-{index}");
            state.navigate(root.clone());
            let request = state.begin().unwrap();
            state.settle(request, Ok(view(&root, 1, false)));
        }
        assert_eq!(state.cache.len(), MAX_CACHED_ROOTS);
    }

    #[test]
    fn back_navigation_stops_at_the_public_root() {
        assert_eq!(parent_root("/Users/name"), Some("/Users".into()));
        assert_eq!(parent_root("/Users"), Some("/".into()));
        assert_eq!(parent_root("/"), None);
    }
}
