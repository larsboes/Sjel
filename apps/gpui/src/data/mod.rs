//! Every read and write against the shell, in one place.
//!
//! A read keeps its last good value while it retries, never runs twice at once for the same
//! slot, and settles into the slot it came from. A write toasts its outcome and then lets the
//! caller re-read only what it changed, so a task edit does not pull the project again.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::WindowExt;
use gpui_kit::{Context, SharedString, Window};

use sjel_gpui_core::shell::{self, Shell, ShellError, ShellResult};

/// Data older than this shows its age beside the page title.
pub const STALE_AFTER: Duration = Duration::from_secs(30);

/// The shell client, or the reason there is none.
#[derive(Clone)]
pub struct Repo {
    shell: Result<Arc<Shell>, ShellError>,
}

impl Repo {
    pub fn new(shell: Result<Arc<Shell>, ShellError>) -> Self {
        Self { shell }
    }

    pub fn shell(&self) -> ShellResult<Arc<Shell>> {
        self.shell.clone()
    }

    pub fn endpoint(&self) -> SharedString {
        match &self.shell {
            Ok(shell) => shell.endpoint().to_owned().into(),
            Err(error) => error.to_string().into(),
        }
    }
}

/// The last answer for one slot. A failed read keeps the value on screen and adds the error.
pub struct Remote<T> {
    pub value: Option<T>,
    pub error: Option<ShellError>,
    pub loaded_at: Option<Instant>,
    pub loading: bool,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            value: None,
            error: None,
            loaded_at: None,
            loading: false,
        }
    }
}

impl<T> Remote<T> {
    pub fn new_loading() -> Self {
        Self {
            loading: true,
            ..Self::default()
        }
    }

    pub fn settle(&mut self, result: ShellResult<T>) {
        self.loading = false;
        match result {
            Ok(value) => {
                self.value = Some(value);
                self.error = None;
                self.loaded_at = Some(Instant::now());
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Neither loaded nor failed yet: the page reserves space and shows no error.
    pub fn first_load(&self) -> bool {
        self.value.is_none() && self.error.is_none()
    }

    /// Seconds since this value arrived, for a value that counts up on its own.
    pub fn age_of_value(&self) -> u64 {
        self.loaded_at.map(|at| at.elapsed().as_secs()).unwrap_or(0)
    }

    /// How long ago this value arrived, once it is past [`STALE_AFTER`].
    pub fn age(&self) -> Option<Duration> {
        self.loaded_at
            .map(|at| at.elapsed())
            .filter(|elapsed| *elapsed > STALE_AFTER)
    }

    /// Marks the slot in flight, or reports that a read is already running.
    fn begin(&mut self) -> bool {
        if self.loading {
            return false;
        }
        self.loading = true;
        true
    }
}

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

/// The writes in flight, by key, so one button cannot be pressed twice.
#[derive(Default)]
pub struct Busy(HashSet<String>);

impl Busy {
    /// Marks `key` in flight, or reports that it already is.
    pub fn start(&mut self, key: &str) -> bool {
        self.0.insert(key.to_owned())
    }

    pub fn finish(&mut self, key: &str) {
        self.0.remove(key);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains(key)
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
