//! The coding tool set: read, write, edit, bash, grep, find.
//!
//! The same six pi gives a model. `grep` and `find` run in-process on ripgrep's own crates
//! (`ignore`, `grep-searcher`, `grep-regex`) instead of spawning a process per call. The walk
//! uses every core and respects `.gitignore`, so a search skips `target/` and `node_modules/`
//! without being told. `read`, `grep` and `find` are read-only, so the loop runs a batch of
//! them in parallel.
//!
//! Paths are relative to the working directory the tools were built with. They are NOT
//! confined to it, the same as pi: `bash` can reach anything the process can, so a path check
//! on the file tools would be decoration. A front end that must confine the model (the
//! assistant capability) does not hand it these tools.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder};
use ignore::overrides::OverrideBuilder;
use ignore::{WalkBuilder, WalkState};
use serde_json::{json, Value};

use crate::Tool;

/// The most text one tool result puts in the context window.
const OUTPUT_LIMIT: usize = 30_000;
const READ_LINE_LIMIT: usize = 2_000;
/// The longest line one grep match shows. Minified files have lines of megabytes.
const LINE_LIMIT: usize = 300;
const BASH_DEFAULT_TIMEOUT: u64 = 120;
const BASH_MAX_TIMEOUT: u64 = 600;

/// The six coding tools, rooted at `root`. `bash` kills its command when `stop` turns true,
/// so pass the same flag the [`crate::Agent`] holds.
pub fn coding(root: &Path, stop: &Arc<AtomicBool>) -> Vec<Box<dyn Tool>> {
    let root = root.to_path_buf();
    vec![
        Box::new(ReadFile(root.clone())),
        Box::new(WriteFile(root.clone())),
        Box::new(EditFile(root.clone())),
        Box::new(Bash(root.clone(), Arc::clone(stop))),
        Box::new(Grep(root.clone())),
        Box::new(Find(root)),
    ]
}

/// Cuts `text` to at most `limit` bytes on a char boundary, and says how much was cut.
pub fn cap(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let end = text.floor_char_boundary(limit);
    format!(
        "{}\n[truncated: {} of {} bytes shown]",
        &text[..end],
        end,
        text.len()
    )
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument `{key}`"))
}

fn resolve(root: &Path, path: &str) -> PathBuf {
    root.join(path)
}

struct ReadFile(PathBuf);
impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read"
    }
    fn description(&self) -> &'static str {
        "Read a text file. Returns at most 2000 lines. Use offset and limit to page through a longer file."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["path"], "properties": {
            "path": { "type": "string", "description": "File path, relative to the working directory or absolute." },
            "offset": { "type": "integer", "description": "First line to return, 1-based. Default 1." },
            "limit": { "type": "integer", "description": "Number of lines to return. Default 2000." }
        }})
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let path = resolve(&self.0, str_arg(args, "path")?);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(READ_LINE_LIMIT as u64);
        let skip = usize::try_from(offset - 1).unwrap_or(usize::MAX);
        let take = usize::try_from(limit)
            .unwrap_or(usize::MAX)
            .min(READ_LINE_LIMIT);
        let total = text.lines().count();
        let mut out: String = text
            .lines()
            .skip(skip)
            .take(take)
            .collect::<Vec<_>>()
            .join("\n");
        let shown_to = skip.saturating_add(take).min(total);
        if shown_to < total {
            out.push_str(&format!(
                "\n[lines {offset}-{shown_to} of {total}; continue with offset {}]",
                shown_to + 1
            ));
        }
        Ok(cap(&out, OUTPUT_LIMIT))
    }
}

struct WriteFile(PathBuf);
impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        "Create or overwrite a file with the given content. Creates missing parent directories."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["path", "content"], "properties": {
            "path": { "type": "string" },
            "content": { "type": "string" }
        }})
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let path = resolve(&self.0, str_arg(args, "path")?);
        let content = str_arg(args, "content")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&path, content).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!(
            "wrote {} bytes to {}",
            content.len(),
            path.display()
        ))
    }
}

struct EditFile(PathBuf);
impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit"
    }
    fn description(&self) -> &'static str {
        "Replace one exact occurrence of old_text with new_text in a file. old_text must match the file byte for byte, including whitespace, and must occur exactly once."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["path", "old_text", "new_text"], "properties": {
            "path": { "type": "string" },
            "old_text": { "type": "string" },
            "new_text": { "type": "string" }
        }})
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let path = resolve(&self.0, str_arg(args, "path")?);
        let old = str_arg(args, "old_text")?;
        let new = str_arg(args, "new_text")?;
        if old.is_empty() {
            return Err("old_text is empty; use write to create a file".into());
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        match text.matches(old).count() {
            0 => Err(format!(
                "old_text does not occur in {}; read the file and copy it exactly",
                path.display()
            )),
            1 => {
                std::fs::write(&path, text.replacen(old, new, 1))
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(format!("edited {}", path.display()))
            }
            n => Err(format!(
                "old_text occurs {n} times in {}; include more surrounding lines so it occurs once",
                path.display()
            )),
        }
    }
}

struct Bash(PathBuf, Arc<AtomicBool>);
impl Tool for Bash {
    fn name(&self) -> &'static str {
        "bash"
    }
    fn description(&self) -> &'static str {
        "Run a shell command in the working directory. Returns the exit code, stdout and stderr. The command is killed after timeout seconds (default 120, maximum 600)."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["command"], "properties": {
            "command": { "type": "string" },
            "timeout": { "type": "integer", "description": "Seconds. Default 120, maximum 600." }
        }})
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let command = str_arg(args, "command")?;
        let timeout = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(BASH_DEFAULT_TIMEOUT)
            .clamp(1, BASH_MAX_TIMEOUT);
        let mut shell = Command::new("bash");
        shell.arg("-c").arg(command).current_dir(&self.0);
        run_bounded(shell, Duration::from_secs(timeout), &self.1)
    }
}

/// Runs `command` with a deadline and returns its exit code and output.
///
/// Public because it is where the kill semantics live, and there is one set of them: the `bash`
/// tool and the guard's `vault_exec` both need a child that dies as a group, is drained on both
/// pipes, and stops with the turn. A second copy in the extension is how the two drift.
///
/// The child leads its own process group, so the timeout or `stop` kills the whole pipeline it
/// started. The group also keeps the terminal's Ctrl-C away from the child: the agent decides
/// what a Ctrl-C stops, and it reaches the child only through `stop`.
/// Killing only the shell would leave a grandchild holding the output pipes open, and the
/// readers below would wait for it forever.
pub fn run_bounded(
    mut command: Command,
    timeout: Duration,
    stop: &AtomicBool,
) -> Result<String, String> {
    use std::os::unix::process::CommandExt as _;
    let mut child = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the command: {e}"))?;
    // One reader per pipe: reading them in turn deadlocks when the other one fills.
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            // Relaxed: the flag carries no data, only "stop", and the poll rereads it.
            Ok(None) if stop.load(Ordering::Relaxed) => {
                kill_group(&mut child);
                break Err("stopped by the user".to_owned());
            }
            Ok(None) if Instant::now() >= deadline => {
                kill_group(&mut child);
                break Err(format!("killed after {} s", timeout.as_secs()));
            }
            // ponytail: 20 ms poll, a pidfd/kqueue wait if a tool call ever needs lower latency
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("could not wait for the command: {e}")),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    let head = match status.map(|s| s.code()) {
        Ok(Some(code)) => format!("exit code {code}"),
        Ok(None) => "killed by a signal".to_owned(),
        Err(why) => why,
    };
    let mut out = head;
    if !stdout.is_empty() {
        out.push_str("\n--- stdout\n");
        out.push_str(&stdout);
    }
    if !stderr.is_empty() {
        out.push_str("\n--- stderr\n");
        out.push_str(&stderr);
    }
    Ok(cap(&out, OUTPUT_LIMIT))
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

fn kill_group(child: &mut Child) {
    // A negative pid addresses the process group. Sending a signal to a group needs libc, which
    // means `unsafe`, and the workspace denies it, so this goes through a shell. The shell's
    // builtin `kill` is used rather than a `kill` binary: the CI runner's PATH does not resolve
    // one, and a missing binary here left the backgrounded child holding the pipes for its full
    // sleep, which the timeout test catches.
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("kill -KILL -{}", child.id()))
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

struct Grep(PathBuf);
impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents with a regex. Skips hidden files and files ignored by .gitignore. Returns path:line:text for each match, sorted by path."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["pattern"], "properties": {
            "pattern": { "type": "string", "description": "A Rust regex." },
            "path": { "type": "string", "description": "File or directory to search. Default: the working directory." },
            "glob": { "type": "string", "description": "Only search files matching this glob, e.g. *.rs" },
            "ignore_case": { "type": "boolean" },
            "literal": { "type": "boolean", "description": "Treat pattern as a literal string, not a regex." }
        }})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let flag = |key| args.get(key).and_then(Value::as_bool) == Some(true);
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(flag("ignore_case"))
            .fixed_strings(flag("literal"))
            .build(str_arg(args, "pattern")?)
            .map_err(|e| e.to_string())?;
        let found = walk_parallel(&self.0, args, |path, shown, searcher| {
            let mut lines = String::new();
            // An unreadable or non-UTF-8 file ends its own search, not the walk.
            let _ = searcher.search_path(
                &matcher,
                path,
                UTF8(|number, line| {
                    let line = line.trim_end();
                    let line = &line[..line.floor_char_boundary(LINE_LIMIT)];
                    lines.push_str(&format!("{shown}:{number}:{line}\n"));
                    Ok(true)
                }),
            );
            lines
        })?;
        Ok(if found.is_empty() {
            "no matches".into()
        } else {
            found
        })
    }
}

struct Find(PathBuf);
impl Tool for Find {
    fn name(&self) -> &'static str {
        "find"
    }
    fn description(&self) -> &'static str {
        "List files by glob. Skips hidden files and files ignored by .gitignore. Sorted by path."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {
            "glob": { "type": "string", "description": "e.g. **/*.toml. Default: every file." },
            "path": { "type": "string", "description": "Directory to list. Default: the working directory." }
        }})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let found = walk_parallel(&self.0, args, |_, shown, _| format!("{shown}\n"))?;
        Ok(if found.is_empty() {
            "no files".into()
        } else {
            found
        })
    }
}

/// Walks `args.path` (default `root`) on every core with ripgrep's walker, honouring
/// `.gitignore` and `args.glob`, and joins what `visit` returns per file, sorted by path.
///
/// `visit` gets the absolute path, the path as the model should see it (relative to `root`
/// when it is inside), and this thread's searcher. The walk stops once the output budget is
/// spent, so a search of a huge tree costs the budget, not the tree.
fn walk_parallel(
    root: &Path,
    args: &Value,
    visit: impl Fn(&Path, &str, &mut Searcher) -> String + Sync,
) -> Result<String, String> {
    let base = resolve(
        root,
        args.get("path").and_then(Value::as_str).unwrap_or("."),
    );
    let mut walk = WalkBuilder::new(&base);
    if let Some(glob) = args.get("glob").and_then(Value::as_str) {
        let mut overrides = OverrideBuilder::new(&base);
        overrides.add(glob).map_err(|e| e.to_string())?;
        walk.overrides(overrides.build().map_err(|e| e.to_string())?);
    }
    let found: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let used = AtomicUsize::new(0);
    walk.build_parallel().run(|| {
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .line_number(true)
            .build();
        let (found, used, visit) = (&found, &used, &visit);
        Box::new(move |entry| {
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            let path = entry.path();
            let shown = path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string();
            let text = visit(path, &shown, &mut searcher);
            if text.is_empty() {
                return WalkState::Continue;
            }
            // Relaxed: the counter only bounds work. The results themselves cross threads
            // through the mutex, which orders them.
            let len = text.len();
            let before = used.fetch_add(len, Ordering::Relaxed);
            found
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((shown, text));
            if before + len > OUTPUT_LIMIT {
                WalkState::Quit
            } else {
                WalkState::Continue
            }
        })
    });
    let mut found = found
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    found.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let joined: String = found.into_iter().map(|(_, text)| text).collect();
    Ok(cap(&joined, OUTPUT_LIMIT))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sjel-agent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool(root: &Path, name: &str) -> Box<dyn Tool> {
        let stop = Arc::new(AtomicBool::new(false));
        coding(root, &stop)
            .into_iter()
            .find(|t| t.name() == name)
            .unwrap()
    }

    #[test]
    fn cap_cuts_on_a_char_boundary() {
        assert_eq!(cap("abc", 3), "abc");
        // "é" is two bytes; a cut at byte 2 lands inside it and must back off to 1.
        assert!(cap("aé", 2).starts_with("a\n[truncated: 1 of 3"));
    }

    #[test]
    fn edit_requires_exactly_one_match() {
        let dir = scratch("edit");
        std::fs::write(dir.join("f.txt"), "one two two").unwrap();
        let edit = tool(&dir, "edit");
        let err = edit
            .run(&json!({ "path": "f.txt", "old_text": "two", "new_text": "x" }))
            .unwrap_err();
        assert!(err.contains("occurs 2 times"));
        assert!(edit
            .run(&json!({ "path": "f.txt", "old_text": "three", "new_text": "x" }))
            .is_err());
        edit.run(&json!({ "path": "f.txt", "old_text": "one", "new_text": "1" }))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("f.txt")).unwrap(),
            "1 two two"
        );
    }

    #[test]
    fn read_pages_and_says_where_to_continue() {
        let dir = scratch("read");
        std::fs::write(dir.join("f.txt"), "a\nb\nc\nd").unwrap();
        let out = tool(&dir, "read")
            .run(&json!({ "path": "f.txt", "offset": 2, "limit": 2 }))
            .unwrap();
        assert_eq!(out, "b\nc\n[lines 2-3 of 4; continue with offset 4]");
    }

    #[test]
    fn grep_and_find_respect_gitignore_and_glob() {
        let dir = scratch("search");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        // `ignore` reads .gitignore only inside a git repository, as rg does.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn needle() {}\n").unwrap();
        std::fs::write(dir.join("target/b.rs"), "fn needle() {}\n").unwrap();
        std::fs::write(dir.join("c.txt"), "x\nNEEDLE\n").unwrap();
        let grep = tool(&dir, "grep");
        let out = grep
            .run(&json!({ "pattern": "needle", "glob": "*.rs" }))
            .unwrap();
        assert_eq!(out, "src/a.rs:1:fn needle() {}\n");
        let out = grep
            .run(&json!({ "pattern": "needle", "ignore_case": true }))
            .unwrap();
        assert_eq!(out, "c.txt:2:NEEDLE\nsrc/a.rs:1:fn needle() {}\n");
        assert_eq!(
            grep.run(&json!({ "pattern": "absent" })).unwrap(),
            "no matches"
        );
        assert!(grep.run(&json!({ "pattern": "(" })).is_err());
        let find = tool(&dir, "find");
        assert_eq!(find.run(&json!({})).unwrap(), "c.txt\nsrc/a.rs\n");
        assert_eq!(find.run(&json!({ "glob": "*.txt" })).unwrap(), "c.txt\n");
    }

    #[test]
    fn bash_reports_exit_code_and_kills_on_timeout() {
        let dir = scratch("bash");
        let bash = tool(&dir, "bash");
        let out = bash
            .run(&json!({ "command": "echo hi; echo oops >&2; exit 3" }))
            .unwrap();
        assert_eq!(out, "exit code 3\n--- stdout\nhi\n\n--- stderr\noops\n");
        // The background sleep holds the pipes; only a group kill lets this return.
        let started = Instant::now();
        let out = bash
            .run(&json!({ "command": "sleep 30 & sleep 30", "timeout": 1 }))
            .unwrap();
        assert!(out.starts_with("killed after 1 s"), "{out}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn bash_stops_when_the_flag_turns_true() {
        let dir = scratch("stop");
        let stop = Arc::new(AtomicBool::new(false));
        let bash = Bash(dir, Arc::clone(&stop));
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            stop.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let out = bash.run(&json!({ "command": "sleep 30" })).unwrap();
        setter.join().unwrap();
        assert_eq!(out, "stopped by the user");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
