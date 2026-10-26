//! Terminals held on this Mac and driven from the app or the phone.
//!
//! A terminal is a full shell with the user account's rights, so the approval gate in `gate.rs`
//! cannot stand in front of it: a keystroke is a command, and there is no card between the two.
//! Access is the control instead — the agent token is refused on every route here (`CLOSED` in
//! `main.rs`), the capability stays on loopback, and the phone arrives the way the rest of Sjel
//! does, through a paired-device signature.
//!
//! The pty itself is `portable-pty`: it is the part that must be a `setsid` plus a controlling
//! terminal, and this workspace denies `unsafe_code`, so the wrapper is the dependency rather than
//! the code. Everything a reader needs is kept here — a ring of the raw output, so a client that
//! reattaches reads from its last byte offset instead of restarting the program.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde_json::{json, Value};

use crate::session::{new_id, now_secs};

/// How much raw output one terminal keeps for a client that reattaches. A shell scrolls past this
/// in a long session; the client is told the oldest offset it can still ask for.
pub const SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;
/// How many terminals may be alive at once. Each is a shell and a thread.
pub const MAX_TERMINALS: usize = 8;
/// A terminal with no client attached and no output for this long is closed like a `DELETE`.
pub const IDLE: Duration = Duration::from_secs(12 * 60 * 60);
/// One server-sent event carries at most this much output, so a flood is a run of frames rather
/// than one frame no client can hold.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// The largest input one request may carry, before base64 decoding.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
/// How long a `DELETE` waits for the process group to go after SIGHUP.
const KILL_GRACE: Duration = Duration::from_secs(5);
/// How often the idle sweep looks. A minute-scale loop over at most [`MAX_TERMINALS`] entries.
const REAP_EVERY: Duration = Duration::from_secs(60 * 10);
/// What a session runs when the overlay names no presets.
const DEFAULT_PRESETS: [(&str, &str); 5] = [
    ("shell", ""),
    ("claude", "claude"),
    ("codex", "codex"),
    ("pi", "pi"),
    ("sjel-agent", "sjel-agent"),
];
/// The parts of the user's environment a login shell may see. An allowlist rather than a filter,
/// because the capability's own environment carries the overlay root and the agent's tokens, and a
/// terminal must inherit neither.
const INHERITED: [&str; 10] = [
    "HOME",
    "USER",
    "LOGNAME",
    "PATH",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "TZ",
    "SHELL",
    "SSH_AUTH_SOCK",
];

/// The presets one machine runs, from `<overlay>/config/agent.toml` under `[terminal.presets]`.
///
/// The list is configuration rather than code because an install path is a machine fact: `pi` may
/// be a Homebrew binary here and a `~/.local/bin` script there.
#[derive(Clone, Debug)]
pub struct Presets {
    commands: Vec<(String, String)>,
}

impl Presets {
    /// The five Sjel starts from, before the overlay says otherwise.
    pub fn defaults() -> Self {
        Self {
            commands: DEFAULT_PRESETS
                .iter()
                .map(|(name, command)| ((*name).to_owned(), (*command).to_owned()))
                .collect(),
        }
    }

    /// [`Presets::defaults`] overlaid with `[terminal.presets]` from `text`.
    ///
    /// A malformed section keeps the defaults rather than leaving a terminal with no preset: a
    /// typo in a config file must not take the shell away.
    pub fn parse(text: &str) -> Self {
        let mut presets = Self::defaults();
        // `toml::from_str`, not `parse`: in toml 1.x a `Value`'s `FromStr` reads one value, not a
        // document (`plugins.rs` records the same).
        let Ok(value) = toml::from_str::<toml::Value>(text) else {
            return presets;
        };
        let Some(table) = value
            .get("terminal")
            .and_then(|terminal| terminal.get("presets"))
            .and_then(toml::Value::as_table)
        else {
            return presets;
        };
        for (name, command) in table {
            let Some(command) = command.as_str() else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let command = command.trim().to_owned();
            match presets
                .commands
                .iter_mut()
                .find(|(existing, _)| existing == name)
            {
                Some(entry) => entry.1 = command,
                None => presets.commands.push((name.to_owned(), command)),
            }
        }
        presets
    }

    /// The presets this machine has, from the overlay, or the defaults.
    pub fn configured() -> Self {
        sjel_config::overlay_config("agent.toml")
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_else(Self::defaults)
    }

    pub fn command(&self, name: &str) -> Option<&str> {
        self.commands
            .iter()
            .find(|(existing, _)| existing == name)
            .map(|(_, command)| command.as_str())
    }

    pub fn names(&self) -> Vec<&str> {
        self.commands
            .iter()
            .map(|(name, _)| name.as_str())
            .collect()
    }
}

impl Default for Presets {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Why a terminal was not opened or not closed. The message is the operator's; the kind is the
/// status the route answers with.
#[derive(Debug)]
pub enum Refusal {
    /// An unknown preset, or a `cwd` that is not a directory under `$HOME`.
    BadRequest(String),
    /// [`MAX_TERMINALS`] are already running.
    TooMany(String),
    /// The pty or the program could not start.
    Failed(String),
    Missing(String),
}

impl Refusal {
    pub fn message(&self) -> &str {
        match self {
            Refusal::BadRequest(message)
            | Refusal::TooMany(message)
            | Refusal::Failed(message)
            | Refusal::Missing(message) => message,
        }
    }
}

/// What one request asks for when it opens a terminal.
pub struct Open<'a> {
    pub preset: &'a str,
    pub cwd: Option<&'a str>,
    pub cols: u16,
    pub rows: u16,
    /// Who opened it: a paired device's name, or `loopback`. Recorded once, in the audit line.
    pub client: &'a str,
}

/// The folder under `$HOME` whose subfolders a new terminal is offered, and how many of them.
const PROJECT_ROOT: &str = "Developer";
const MAX_FOLDERS: usize = 20;

/// Where a new terminal is offered to start: `home`, then the folders in `~/Developer`, most
/// recently changed first. Hidden folders and files are not offered.
pub fn folders(home: &Path) -> Vec<String> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> =
        std::fs::read_dir(home.join(PROJECT_ROOT))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    std::iter::once(home.to_path_buf())
        .chain(found.into_iter().take(MAX_FOLDERS).map(|(_, path)| path))
        .map(|path| path.display().to_string())
        .collect()
}

/// The terminals this process holds.
pub struct Terminals {
    live: Mutex<HashMap<String, Arc<Terminal>>>,
    audit: PathBuf,
    presets: Presets,
}

impl Terminals {
    pub fn new(audit: PathBuf, presets: Presets) -> Self {
        Self {
            live: Mutex::new(HashMap::new()),
            audit,
            presets,
        }
    }

    /// The preset names a client may open, in the overlay's order.
    pub fn preset_names(&self) -> Vec<String> {
        self.presets
            .names()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Terminal>> {
        self.lock().get(id).map(Arc::clone)
    }

    /// Newest first, the way the session list reads.
    pub fn list(&self) -> Vec<Value> {
        let mut listed: Vec<Value> = self.lock().values().map(|t| t.meta()).collect();
        listed.sort_by_key(|meta| std::cmp::Reverse(meta["started_at"].as_i64().unwrap_or(0)));
        listed
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn open(&self, request: Open<'_>) -> Result<Arc<Terminal>, Refusal> {
        let preset = request.preset.trim();
        let command = self
            .presets
            .command(preset)
            .map(str::to_owned)
            .ok_or_else(|| {
                Refusal::BadRequest(format!(
                    "unknown preset `{preset}`. This machine has: {}",
                    self.presets.names().join(", ")
                ))
            })?;
        let cwd = working_directory(request.cwd)?;
        if self.len() >= MAX_TERMINALS {
            return Err(Refusal::TooMany(format!(
                "{MAX_TERMINALS} terminals are already open; close one first"
            )));
        }

        let size = PtySize {
            rows: clamp_rows(request.rows),
            cols: clamp_cols(request.cols),
            pixel_width: 0,
            pixel_height: 0,
        };
        let system = native_pty_system();
        let pair = system
            .openpty(size)
            .map_err(|error| Refusal::Failed(format!("no pty: {error}")))?;
        let builder = command_builder(&command, &cwd);
        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|error| Refusal::Failed(format!("{preset} did not start: {error}")))?;
        drop(pair.slave);
        let master = pair.master;
        // The child is a session leader, so its process id is the id of the group a `DELETE`
        // signals. Asking the pty is how a group is named without a libc call of our own.
        #[cfg(unix)]
        let group = master.process_group_leader();
        #[cfg(not(unix))]
        let group: Option<i32> = None;
        let writer = master
            .take_writer()
            .map_err(|error| Refusal::Failed(format!("no writer: {error}")))?;
        let reader = master
            .try_clone_reader()
            .map_err(|error| Refusal::Failed(format!("no reader: {error}")))?;
        let killer = child.clone_killer();

        let terminal = Arc::new(Terminal {
            id: new_id(),
            preset: preset.to_owned(),
            title: title_of(&command),
            cwd: cwd.clone(),
            started_at: now_secs() as i64,
            last_output_at: AtomicI64::new(now_secs() as i64),
            clients: AtomicUsize::new(0),
            output: Mutex::new(Ring::new()),
            subscribers: Subscribers::default(),
            master: Mutex::new(master),
            writer: Mutex::new(writer),
            killer,
            group,
            exited: AtomicBool::new(false),
            status: Mutex::new(None),
            audit: self.audit.clone(),
            started_by: request.client.to_owned(),
        });
        terminal.audit_line("start", None);

        let reading = Arc::clone(&terminal);
        let spawned = std::thread::Builder::new()
            .name(format!("terminal-{}", terminal.id))
            .spawn(move || {
                let mut child = child;
                let mut reader = reader;
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => reading.append(&buffer[..read]),
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                // The master sees EOF when the program and everything it started are gone, which
                // is the moment `wait` returns without blocking.
                let status = child.wait().ok();
                reading.finish(status);
            });
        if let Err(error) = spawned {
            terminal.signal("KILL");
            return Err(Refusal::Failed(format!(
                "the reader thread did not start: {error}"
            )));
        }

        self.lock()
            .insert(terminal.id.clone(), Arc::clone(&terminal));
        Ok(terminal)
    }

    /// Ends the terminal named by `id`: SIGHUP to its process group, SIGKILL after the grace.
    ///
    /// `Missing` when no terminal has that id, so the route can answer 404 rather than pretend.
    pub async fn close(&self, id: &str) -> Result<(), Refusal> {
        let terminal = self.lock().remove(id);
        let Some(terminal) = terminal else {
            return Err(Refusal::Missing("no such terminal".to_owned()));
        };
        if !terminal.exited.load(Ordering::SeqCst) {
            terminal.signal("HUP");
            let deadline = std::time::Instant::now() + KILL_GRACE;
            while std::time::Instant::now() < deadline {
                if terminal.exited.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if !terminal.exited.load(Ordering::SeqCst) {
                terminal.signal("KILL");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        Ok(())
    }

    /// Closes what nobody has looked at for [`IDLE`]. Returns how many ended.
    pub async fn reap(&self) -> usize {
        let now = now_secs() as i64;
        let idle: Vec<String> = self
            .lock()
            .iter()
            .filter(|(_, terminal)| {
                terminal.clients.load(Ordering::SeqCst) == 0
                    && now - terminal.last_output_at.load(Ordering::SeqCst) >= IDLE.as_secs() as i64
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut closed = 0;
        for id in idle {
            if self.close(&id).await.is_ok() {
                closed += 1;
            }
        }
        closed
    }

    /// The sweep [`REAP_EVERY`] describes, for the process rather than for a test.
    pub async fn reap_forever(state: Arc<Self>) {
        loop {
            tokio::time::sleep(REAP_EVERY).await;
            state.reap().await;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Terminal>>> {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One running program, its output ring, and whoever is watching it.
pub struct Terminal {
    pub id: String,
    pub preset: String,
    pub title: String,
    pub cwd: PathBuf,
    started_at: i64,
    last_output_at: AtomicI64,
    clients: AtomicUsize,
    output: Mutex<Ring>,
    subscribers: Subscribers,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// The session leader, whose id is also its process group's. `None` when the pty would not
    /// name one, and then a `DELETE` signals the child alone.
    group: Option<i32>,
    exited: AtomicBool,
    status: Mutex<Option<End>>,
    audit: PathBuf,
    started_by: String,
}

/// How a terminal ended.
#[derive(Clone, Debug, PartialEq, Eq)]
struct End {
    code: i32,
    signal: Option<String>,
}

impl Terminal {
    pub fn alive(&self) -> bool {
        !self.exited.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn exit_code(&self) -> Option<i32> {
        self.lock_status().as_ref().map(|end| end.code)
    }

    /// The list row: what the phone needs to show a terminal it is not attached to.
    pub fn meta(&self) -> Value {
        let end = self.lock_status();
        json!({
            "id": self.id,
            "preset": self.preset,
            "title": self.title,
            "cwd": self.cwd.to_string_lossy(),
            "started_at": self.started_at,
            "last_output_at": self.last_output_at.load(Ordering::SeqCst),
            "alive": end.is_none(),
            "exit_code": end.as_ref().map(|end| end.code),
            "exit_signal": end.as_ref().and_then(|end| end.signal.clone()),
            "clients": self.clients.load(Ordering::SeqCst),
        })
    }

    /// The byte offset a fresh client starts from: the oldest output still held.
    pub fn first_offset(&self) -> u64 {
        self.lock_output().first()
    }

    /// What one reader has not seen yet, or the oldest offset it can still ask for.
    pub fn read(&self, from: u64, max: usize) -> ReadOutcome {
        let output = self.lock_output();
        if from < output.first() {
            return ReadOutcome::Truncated {
                first_offset: output.first(),
            };
        }
        let (offset, bytes) = output.read(from, max);
        ReadOutcome::Output { offset, bytes }
    }

    pub fn subscribe(&self) -> tokio::sync::mpsc::Receiver<()> {
        self.subscribers.subscribe()
    }

    pub fn attach(&self) {
        self.clients.fetch_add(1, Ordering::SeqCst);
    }

    pub fn detach(&self) {
        // A close racing a detach must not wrap the count around to a huge number.
        let _ = self
            .clients
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |clients| {
                Some(clients.saturating_sub(1))
            });
    }

    pub fn write(&self, bytes: &[u8]) -> Result<(), String> {
        if !self.alive() {
            return Err("the terminal has ended".to_owned());
        }
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(|error| format!("the terminal refused input: {error}"))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let size = PtySize {
            rows: clamp_rows(rows),
            cols: clamp_cols(cols),
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .resize(size)
            .map_err(|error| format!("the pty did not resize: {error}"))
    }

    /// The kernel's own idea of the size, so a test can see that a resize landed.
    #[cfg(test)]
    pub fn size(&self) -> Option<(u16, u16)> {
        self.master
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_size()
            .ok()
            .map(|size| (size.cols, size.rows))
    }

    /// SIGHUP or SIGKILL to the session's process group.
    ///
    /// `/bin/kill` rather than a libc call: a group is signalled with `kill(2)` on a negative id,
    /// and this workspace denies `unsafe_code`. The fallback is the child alone, which still ends
    /// the shell even when the pty never named its group.
    fn signal(&self, name: &str) {
        let signalled = self.group.is_some_and(|group| {
            std::process::Command::new("/bin/kill")
                .arg(format!("-{name}"))
                .arg(format!("-{group}"))
                .status()
                .is_ok_and(|status| status.success())
        });
        if !signalled {
            let mut killer = self.killer.clone_killer();
            let _ = killer.kill();
        }
    }

    fn append(&self, data: &[u8]) {
        self.lock_output().append(data);
        self.last_output_at
            .store(now_secs() as i64, Ordering::SeqCst);
        self.subscribers.wake();
    }

    fn finish(&self, status: Option<portable_pty::ExitStatus>) {
        let end = match status {
            Some(status) => End {
                code: status.exit_code() as i32,
                signal: status.signal().map(str::to_owned),
            },
            // A reader that could not wait still saw the end of the output; the code is unknown.
            None => End {
                code: 0,
                signal: None,
            },
        };
        *self.lock_status() = Some(end.clone());
        self.exited.store(true, Ordering::SeqCst);
        self.audit_line("end", Some(end.code));
        self.subscribers.wake();
    }

    /// One audit line per start and per end: no input, no output, nothing a terminal displayed.
    fn audit_line(&self, event: &str, exit_code: Option<i32>) {
        let line = json!({
            "at": now_secs() as i64,
            "id": self.id,
            "preset": self.preset,
            "cwd": self.cwd.to_string_lossy(),
            "client": self.started_by,
            "event": event,
            "exit_code": exit_code,
        });
        let Some(parent) = self.audit.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.audit)
        else {
            return;
        };
        let _ = writeln!(file, "{line}");
    }

    fn lock_output(&self) -> std::sync::MutexGuard<'_, Ring> {
        self.output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_status(&self) -> std::sync::MutexGuard<'_, Option<End>> {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// What a reader asked for and what the ring could give it.
#[derive(Debug)]
pub enum ReadOutcome {
    /// The offset is older than the scrollback. The client clears and reads again from here.
    Truncated {
        first_offset: u64,
    },
    Output {
        offset: u64,
        bytes: Vec<u8>,
    },
}

/// The events a stream ended on, so the handler can close the response.
pub fn exit_event(terminal: &Terminal) -> Value {
    let end = terminal.lock_status().clone();
    match end {
        Some(end) => json!({ "code": end.code, "signal": end.signal }),
        None => json!({ "code": 0, "signal": Value::Null }),
    }
}

/// The offsets of the retained output. Bytes leave the front when the ring is full, so `first`
/// moves and a reader that fell behind is told rather than handed a hole.
struct Ring {
    bytes: Vec<u8>,
    first: u64,
}

impl Ring {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            first: 0,
        }
    }

    fn first(&self) -> u64 {
        self.first
    }

    #[cfg(test)]
    fn end(&self) -> u64 {
        self.first + self.bytes.len() as u64
    }

    fn append(&mut self, data: &[u8]) {
        if data.len() >= SCROLLBACK_BYTES {
            self.first += (self.bytes.len() + data.len() - SCROLLBACK_BYTES) as u64;
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&data[data.len() - SCROLLBACK_BYTES..]);
            return;
        }
        self.bytes.extend_from_slice(data);
        let excess = self.bytes.len().saturating_sub(SCROLLBACK_BYTES);
        if excess > 0 {
            self.bytes.drain(..excess);
            self.first += excess as u64;
        }
    }

    /// Bytes at `from`, at most `max` of them. An empty result means nothing new arrived.
    fn read(&self, from: u64, max: usize) -> (u64, Vec<u8>) {
        let from = from.max(self.first);
        let start = (from - self.first) as usize;
        let end = (start + max.min(MAX_FRAME_BYTES)).min(self.bytes.len());
        if start >= end {
            return (from, Vec::new());
        }
        (from, self.bytes[start..end].to_vec())
    }
}

/// A reader's wake, done as a one-slot channel per subscriber: the thread that reads the pty never
/// waits for a client.
#[derive(Default)]
struct Subscribers(Mutex<Vec<tokio::sync::mpsc::Sender<()>>>);

impl Subscribers {
    fn subscribe(&self) -> tokio::sync::mpsc::Receiver<()> {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(sender);
        receiver
    }

    fn wake(&self) {
        let mut subscribers = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        subscribers.retain(|sender| sender.try_send(()).is_ok() || !sender.is_closed());
    }
}

/// The login shell, then a preset's command inside it so `PATH` matches an interactive terminal.
fn command_builder(command: &str, cwd: &Path) -> CommandBuilder {
    let shell = login_shell();
    let mut builder = CommandBuilder::new(&shell);
    builder.arg("-l");
    if !command.is_empty() {
        builder.arg("-c");
        builder.arg(command);
    }
    // Nothing from the capability's own environment: it carries the overlay root and the tokens
    // an agent would need. The allowlist is what a login shell would have had anyway.
    builder.env_clear();
    for key in INHERITED {
        if let Some(value) = std::env::var_os(key) {
            builder.env(key, value);
        }
    }
    builder.env("SHELL", &shell);
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.cwd(cwd);
    builder
}

fn login_shell() -> String {
    let candidates = [
        std::env::var("SHELL").unwrap_or_default(),
        "/bin/zsh".to_owned(),
        "/bin/bash".to_owned(),
        "/bin/sh".to_owned(),
    ];
    candidates
        .into_iter()
        .find(|path| !path.is_empty() && Path::new(path).is_file())
        .unwrap_or_else(|| "/bin/sh".to_owned())
}

/// A `cwd` the terminal may start in: an existing directory under `$HOME`.
fn working_directory(cwd: Option<&str>) -> Result<PathBuf, Refusal> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Refusal::Failed("no HOME in this environment".to_owned()))?;
    let home = std::fs::canonicalize(&home).unwrap_or(home);
    let asked = match cwd.map(str::trim).filter(|cwd| !cwd.is_empty()) {
        Some(asked) => expand(asked),
        None => home.clone(),
    };
    let resolved = std::fs::canonicalize(&asked)
        .map_err(|error| Refusal::BadRequest(format!("{}: {error}", asked.to_string_lossy())))?;
    if !resolved.is_dir() {
        return Err(Refusal::BadRequest(format!(
            "{} is not a directory",
            resolved.to_string_lossy()
        )));
    }
    if !resolved.starts_with(&home) {
        return Err(Refusal::BadRequest(format!(
            "{} is outside {}; a terminal starts under home",
            resolved.to_string_lossy(),
            home.to_string_lossy()
        )));
    }
    Ok(resolved)
}

/// `~` and `~/…`, so a directory can be typed the way it is written.
fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(path)
}

/// What names the terminal in the list: the program it started, not its arguments.
fn title_of(command: &str) -> String {
    if command.trim().is_empty() {
        return Path::new(&login_shell())
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "shell".to_owned());
    }
    command
        .split_whitespace()
        .next()
        .map(|program| {
            Path::new(program)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| program.to_owned())
        })
        .unwrap_or_else(|| command.to_owned())
}

fn clamp_cols(cols: u16) -> u16 {
    cols.clamp(1, 1_000)
}

fn clamp_rows(rows: u16) -> u16 {
    rows.clamp(1, 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_offer_home_then_project_folders_newest_first() {
        let home = std::env::temp_dir().join(format!("sjel-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let root = home.join(PROJECT_ROOT);
        std::fs::create_dir_all(root.join("older")).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::create_dir_all(root.join("newer")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("notes.txt"), "a file").unwrap();
        let offered = folders(&home);
        let _ = std::fs::remove_dir_all(&home);
        let names: Vec<&str> = offered
            .iter()
            .map(|path| path.rsplit('/').next().unwrap_or(""))
            .collect();
        assert_eq!(offered[0], home.display().to_string());
        assert_eq!(&names[1..], ["newer", "older"]);
    }

    fn audit_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sjel-terminal-{name}-{}-{}",
            std::process::id(),
            now_secs()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("terminals.jsonl")
    }

    fn read_until<F>(terminal: &Terminal, mut done: F) -> String
    where
        F: FnMut(&str) -> bool,
    {
        let mut seen = String::new();
        let mut cursor = terminal.first_offset();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let ReadOutcome::Output { offset, bytes } = terminal.read(cursor, MAX_FRAME_BYTES) {
                if !bytes.is_empty() {
                    cursor = offset + bytes.len() as u64;
                    seen.push_str(&String::from_utf8_lossy(&bytes));
                    if done(&seen) {
                        return seen;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the terminal never printed what the test waited for: {seen}");
    }

    fn wait_until_exited(terminal: &Terminal) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while terminal.alive() {
            assert!(
                std::time::Instant::now() < deadline,
                "the terminal outlived its program"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn run(name: &str, command: &str) -> (Terminals, Arc<Terminal>) {
        let presets = Presets::parse(&format!("[terminal.presets]\necho = {command:?}\n"));
        let terminals = Terminals::new(audit_path(name), presets);
        let cwd = std::env::var("HOME").unwrap();
        let terminal = terminals
            .open(Open {
                preset: "echo",
                cwd: Some(&cwd),
                cols: 80,
                rows: 24,
                client: "test",
            })
            .expect("the terminal starts");
        (terminals, terminal)
    }

    #[test]
    fn the_overlay_presets_win_and_the_defaults_stay() {
        let presets = Presets::parse(
            "[terminal.presets]\nclaude = \"/opt/bin/claude --resume\"\nmine = \"mine\"\n",
        );
        assert_eq!(presets.command("claude"), Some("/opt/bin/claude --resume"));
        assert_eq!(presets.command("mine"), Some("mine"));
        assert_eq!(presets.command("shell"), Some(""));
        assert!(presets.names().contains(&"codex"));

        // A typo in the file keeps the defaults rather than taking the shell away.
        let broken = Presets::parse("[terminal.presets\nshell = ");
        assert_eq!(broken.command("shell"), Some(""));
    }

    #[test]
    fn a_directory_outside_home_is_refused() {
        let outside = std::env::temp_dir();
        let home = std::fs::canonicalize(std::env::var("HOME").unwrap()).unwrap();
        if outside.starts_with(&home) {
            return; // A temporary directory under home proves nothing about the boundary.
        }
        let message = working_directory(Some(&outside.to_string_lossy()))
            .expect_err("a terminal may not start outside home");
        assert!(message.message().contains("outside"), "{message:?}");
    }

    #[test]
    fn the_ring_keeps_the_scrollback_and_moves_the_first_offset() {
        let mut ring = Ring::new();
        assert_eq!(ring.first(), 0);
        ring.append(b"hello");
        assert_eq!(ring.end(), 5);
        assert_eq!(ring.read(0, 100), (0, b"hello".to_vec()));

        // One write larger than the ring leaves only its tail, and says where the tail starts.
        let big = vec![b'x'; SCROLLBACK_BYTES + 10];
        ring.append(&big);
        assert_eq!(ring.first(), 5 + 10);
        assert_eq!(ring.end(), 5 + 10 + SCROLLBACK_BYTES as u64);
        let (_, tail) = ring.read(ring.first(), 16);
        assert_eq!(tail.len(), 16);
    }

    #[test]
    fn a_terminal_runs_a_program_and_its_output_is_read_by_offset() {
        let (_terminals, terminal) = run("echo", "printf 'first line\\nsecond line\\n'");
        let seen = read_until(&terminal, |seen| seen.contains("second line"));
        assert!(seen.contains("first line"), "{seen}");

        // Reading the same offset twice gives the same bytes: an offset is a position, not a
        // cursor the terminal keeps on a reader's behalf.
        let ReadOutcome::Output { bytes, .. } = terminal.read(terminal.first_offset(), 10) else {
            panic!("the first bytes are still held");
        };
        assert_eq!(bytes.len(), 10);
    }

    #[test]
    fn a_terminal_does_not_inherit_the_capabilitys_environment() {
        std::env::set_var("SJEL_TERMINAL_TEST_TOKEN", "must-not-travel");
        let (_terminals, terminal) = run("env", "env");
        let seen = read_until(&terminal, |seen| seen.contains("PATH="));
        assert!(
            !seen.contains("SJEL_TERMINAL_TEST_TOKEN"),
            "the capability's environment reached the shell: {seen}"
        );
        assert!(seen.contains("TERM=xterm-256color"), "{seen}");
        std::env::remove_var("SJEL_TERMINAL_TEST_TOKEN");
    }

    #[test]
    fn a_resize_reaches_the_kernel() {
        let (_terminals, terminal) = run("resize", "sleep 5");
        terminal.resize(120, 40).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if terminal.size() == Some((120, 40)) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the pty kept its old size: {:?}",
                terminal.size()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[tokio::test]
    async fn a_delete_ends_the_process_group_and_audits_a_start_and_an_end() {
        let (terminals, terminal) = run("delete", "sleep 30");
        let id = terminal.id.clone();
        // The shell holds the program, so ending the group means both are gone.
        terminals.close(&id).await.unwrap();
        assert!(!terminal.alive(), "the terminal is still running");
        assert!(terminal.exit_code().is_some());

        let audit = std::fs::read_to_string(&terminals.audit).unwrap();
        let lines: Vec<Value> = audit
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "{audit}");
        assert_eq!(lines[0]["event"], "start");
        assert_eq!(lines[0]["client"], "test");
        assert_eq!(lines[1]["event"], "end");
        assert_eq!(lines[1]["id"], id.as_str());
        // An audit line says where and what, never what was typed or printed.
        assert!(lines[1].get("input").is_none() && lines[1].get("output").is_none());
        assert!(
            terminals.get(&id).is_none(),
            "a closed terminal stays listed"
        );
    }

    #[tokio::test]
    async fn the_limit_and_the_idle_sweep_are_enforced() {
        let presets = Presets::parse("[terminal.presets]\nsleepy = \"sleep 30\"\n");
        let terminals = Terminals::new(audit_path("limit"), presets);
        let cwd = std::env::var("HOME").unwrap();
        let mut open = Vec::new();
        for _ in 0..MAX_TERMINALS {
            open.push(
                terminals
                    .open(Open {
                        preset: "sleepy",
                        cwd: Some(&cwd),
                        cols: 80,
                        rows: 24,
                        client: "test",
                    })
                    .unwrap(),
            );
        }
        let refused = match terminals.open(Open {
            preset: "sleepy",
            cwd: Some(&cwd),
            cols: 80,
            rows: 24,
            client: "test",
        }) {
            Ok(_) => panic!("the ninth terminal was accepted"),
            Err(refusal) => refusal,
        };
        assert!(matches!(refused, Refusal::TooMany(_)), "{refused:?}");

        // Idle means no client and no output for twelve hours; the sweep takes everything here
        // because the clock is moved rather than waited for.
        for terminal in &open {
            terminal.last_output_at.store(
                now_secs() as i64 - IDLE.as_secs() as i64 - 1,
                Ordering::SeqCst,
            );
        }
        assert_eq!(terminals.reap().await, MAX_TERMINALS);
        assert_eq!(terminals.len(), 0);
    }

    #[tokio::test]
    async fn a_closed_terminal_refuses_input_and_an_unknown_id_is_missing() {
        let (terminals, terminal) = run("input", "sleep 30");
        terminal.write(b"echo hi\n").unwrap();
        terminals.close(&terminal.id).await.unwrap();
        assert!(terminal.write(b"x").is_err(), "a dead pty took input");
        assert!(matches!(
            terminals.close(&terminal.id).await,
            Err(Refusal::Missing(_))
        ));
    }

    #[tokio::test]
    async fn a_short_command_ends_on_its_own_and_says_so() {
        let (_terminals, terminal) = run("exit", "echo bye");
        read_until(&terminal, |seen| seen.contains("bye"));
        wait_until_exited(&terminal);
        assert_eq!(terminal.exit_code(), Some(0));
        assert!(!terminal.alive());
        assert!(matches!(
            terminal.read(terminal.first_offset() + 10_000, 16),
            ReadOutcome::Output { bytes, .. } if bytes.is_empty()
        ));
    }

    #[test]
    fn the_list_names_what_the_phone_needs_to_choose_a_terminal() {
        let (terminals, terminal) = run("list", "sleep 5");
        terminal.attach();
        let listed = terminals.list();
        assert_eq!(listed.len(), 1);
        for key in [
            "id",
            "preset",
            "title",
            "cwd",
            "started_at",
            "last_output_at",
            "alive",
            "exit_code",
            "clients",
        ] {
            assert!(listed[0].get(key).is_some(), "no {key} in {}", listed[0]);
        }
        assert_eq!(listed[0]["clients"], 1);
        assert_eq!(listed[0]["alive"], true);
        terminal.detach();
        assert_eq!(terminals.list()[0]["clients"], 0);
    }
}
