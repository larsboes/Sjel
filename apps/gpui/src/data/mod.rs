//! The half of the data layer that needs GPUI: one spawn per read, one toast per write.
//!
//! The adapters themselves live in `sjel_gpui_core::data`, so the iOS app drives the same
//! `Repo`, `Remote`, `Busy`, `Notes` and `Board` without a window or a `Context`. What stays
//! here is what a `Context` is needed for: the future is spawned onto GPUI's executor and its
//! answer settles into the slot the page draws, and a write shows its outcome as a notification.
//!
//! `Press` stays here too, because it is the row a write marks busy — the last thing a caller
//! names before the toast, and the only part of a write that a phone would wire differently.

use std::future::Future;
use std::sync::Arc;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt;
use gpui_kit::{Context, Window};

use sjel_gpui_core::shell::{self, Shell, ShellError, ShellResult};

pub use sjel_gpui_core::data::{Busy, Notes, Remote, Repo};

/// The button one write belongs to: the row it marks busy while it runs, and the sentence a
/// success shows.
pub struct Press<C> {
    slot: Option<fn(&mut C) -> &mut Busy>,
    key: String,
    done: &'static str,
}

impl<C> Press<C> {
    /// A press on a row: it cannot fire again until the answer arrives.
    pub fn once(slot: fn(&mut C) -> &mut Busy, key: impl Into<String>, done: &'static str) -> Self {
        Self {
            slot: Some(slot),
            key: key.into(),
            done,
        }
    }

    /// A write with no row of its own.
    pub fn quiet(done: &'static str) -> Self {
        Self {
            slot: None,
            key: String::new(),
            done,
        }
    }
}

/// Reads into `slot` from the shell, unless that slot is already loading.
pub fn read<C, T, F, Fut>(
    cx: &mut Context<C>,
    this: &mut C,
    slot: fn(&mut C) -> &mut Remote<T>,
    repo: &Repo,
    request: F,
) where
    C: 'static,
    T: Send + 'static,
    F: FnOnce(Arc<Shell>) -> Fut,
    Fut: Future<Output = ShellResult<T>> + Send + 'static,
{
    let shell = match repo.shell() {
        Ok(shell) => shell,
        Err(error) => {
            slot(this).settle(Err(error));
            cx.notify();
            return;
        }
    };
    if !slot(this).begin() {
        return;
    }
    cx.notify();
    let work = shell::run(request(shell));
    cx.spawn(async move |weak, cx| {
        let result = work.await.unwrap_or(Err(ShellError::Unreachable));
        let _ = weak.update(cx, |this, cx| {
            slot(this).settle(result);
            cx.notify();
        });
    })
    .detach();
}

/// Reads into `slot` via `Repo` (which can use direct local DB or Shell fallback),
/// unless that slot is already loading.
pub fn read_repo<C, T, F, Fut>(
    cx: &mut Context<C>,
    this: &mut C,
    slot: fn(&mut C) -> &mut Remote<T>,
    repo: &Repo,
    request: F,
) where
    C: 'static,
    T: Send + 'static,
    F: FnOnce(Repo) -> Fut,
    Fut: Future<Output = ShellResult<T>> + Send + 'static,
{
    if !slot(this).begin() {
        return;
    }
    cx.notify();
    let repo_clone = repo.clone();
    let work = shell::run(request(repo_clone));
    cx.spawn(async move |weak, cx| {
        let result = work.await.unwrap_or(Err(ShellError::Unreachable));
        let _ = weak.update(cx, |this, cx| {
            slot(this).settle(result);
            cx.notify();
        });
    })
    .detach();
}

/// Runs one write, toasts its outcome, and then lets the caller apply what came back.
///
/// `then` receives the shell's answer, or none when the write failed, so a caller can patch one
/// row instead of reading again.
pub fn write<C, T, F, Fut>(
    cx: &mut Context<C>,
    window: &mut Window,
    this: &mut C,
    repo: &Repo,
    press: Press<C>,
    request: F,
    then: fn(&mut C, Option<T>, &mut Context<C>),
) where
    C: 'static,
    T: Send + 'static,
    F: FnOnce(Arc<Shell>) -> Fut,
    Fut: Future<Output = ShellResult<T>> + Send + 'static,
{
    let shell = match repo.shell() {
        Ok(shell) => shell,
        Err(error) => {
            window.push_notification(Notification::error(error.to_string()), cx);
            return;
        }
    };
    let Press { slot, key, done } = press;
    if let Some(slot) = slot {
        if !slot(this).start(&key) {
            return;
        }
    }
    cx.notify();
    let work = shell::run(request(shell));
    cx.spawn_in(window, async move |weak, cx| {
        let result = work.await.unwrap_or(Err(ShellError::Unreachable));
        let _ = weak.update_in(cx, |this, window, cx| {
            if let Some(slot) = slot {
                slot(this).finish(&key);
            }
            match result {
                Ok(value) => {
                    window.push_notification(Notification::success(done), cx);
                    then(this, Some(value), cx);
                }
                Err(error) => {
                    window.push_notification(Notification::error(error.to_string()), cx);
                    then(this, None, cx);
                }
            }
            cx.notify();
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Repo` lives in `core`, so `cfg(test)` there cannot see this test: the app enables core's
    /// `test-support` feature from its dev-dependencies instead. Without that line these window
    /// tests read this machine's real `axon.db`, which made them slow and flaky rather than red.
    #[test]
    fn these_tests_read_no_local_database() {
        assert!(
            Repo::local_db_path().is_none(),
            "a test build must not resolve a database path"
        );
    }
}
