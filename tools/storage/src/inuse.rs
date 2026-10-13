//! Who has this directory open?
//!
//! `prune` printed a size and named `cargo clean`, and neither answers the question that
//! decides whether running it is safe. On 2026-10-12 `~/.cargo-target` held 17.5 GB with
//! `release/sjel-mcp` — the binary this session's MCP server is exec'ing — inside it, and a
//! `cargo check` from another session writing into `debug/deps` at the same time. `cargo
//! clean` there frees the number and breaks both.
//!
//! It is not a hypothetical act. `tools/doctor` names `sjel storage prune --target` as the
//! remediation when the toolchain moves, and `--target` resolves through `CARGO_TARGET_DIR`,
//! so on this machine it means that shared directory rather than anything inside the repo.
//!
//! Three answers, and the third is why this is not a boolean. A probe that could not run
//! must not read as "nothing is using it", because that is the answer that licenses
//! deletion — the rule `service.rs` follows for a service whose probe is missing.

use std::path::Path;
use std::process::Command;

/// A process with a file open under the directory that was asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub command: String,
    /// The descriptor lsof named: `txt` for an executable image, `reg` for an open file,
    /// `cwd` for a working directory. Printed rather than filtered on, because "a build is
    /// writing here" and "a shell happens to be sitting here" are the reader's to weigh.
    pub kind: String,
    pub file: String,
}

/// What the machine answered about one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Usage {
    /// Nothing has a file open under it.
    Free,
    /// These processes do. Removing it breaks them, or the build they are in.
    Held(Vec<Holder>),
    /// The probe could not run. Never `Free` — see the module note.
    Unverifiable(String),
}

/// lsof's `-F pcn` field output into one holder per process.
///
/// Pure, so the parse is tested rather than lived in the command. lsof emits one block per
/// process — `p<pid>`, `c<command>`, then an `f<fd>`/`n<name>` pair per open file — so only
/// the first pair is kept: one line per process is enough to say why the directory is busy.
pub fn holders(stdout: &str) -> Vec<Holder> {
    let mut out: Vec<Holder> = Vec::new();
    let mut pid: Option<u32> = None;
    let mut command = String::new();
    let mut kind = String::new();
    let mut first_file = false;
    for line in stdout.lines() {
        // The bytes are read before the char, and the value through `get`, so a line lsof
        // could not have written is skipped rather than panicking on a char boundary.
        let (tag, value) = match line.as_bytes().first() {
            Some(&tag) => (tag, line.get(1..).unwrap_or_default()),
            None => continue,
        };
        match tag {
            b'p' => {
                pid = value.parse().ok();
                command.clear();
                kind.clear();
                first_file = true;
            }
            b'c' => command = value.to_string(),
            b'f' => kind = value.to_string(),
            b'n' if first_file => {
                if let Some(pid) = pid {
                    out.push(Holder {
                        pid,
                        command: command.clone(),
                        kind: kind.clone(),
                        file: value.to_string(),
                    });
                    first_file = false;
                }
            }
            _ => {}
        }
    }
    out
}

/// Ask the machine. Never panics, never fails a run: an unaskable question is an answer.
///
/// `+D` descends, so anything under the directory counts — an executable in `release/` and a
/// half-written object file three levels down are both reasons to stop.
pub fn usage(dir: &Path) -> Usage {
    let out = match Command::new("lsof")
        .arg("-F")
        .arg("pcn")
        .arg("+D")
        .arg(dir)
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Usage::Unverifiable("no `lsof` on PATH".to_string())
        }
        Err(e) => return Usage::Unverifiable(e.to_string()),
    };
    let found = holders(&String::from_utf8_lossy(&out.stdout));
    if !found.is_empty() {
        return Usage::Held(found);
    }
    // lsof exits 1 whether or not it found anything, so the exit status is not the answer
    // here — stdout is. stderr is the other half: a version banner prints on error, and a
    // message on it means the walk did not finish, which is not the same as an empty one.
    let err = String::from_utf8_lossy(&out.stderr);
    match err.lines().find(|l| l.starts_with("lsof:")) {
        Some(line) => Usage::Unverifiable(line.to_string()),
        None => Usage::Free,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn one_block_per_process_becomes_one_holder() {
        let stdout = concat!(
            "p9720\n",
            "csjel-mcp\n",
            "ftxt\n",
            "n/Users/x/.cargo-target/release/sjel-mcp\n",
            "fmem\n",
            "n/Users/x/.cargo-target/release/deps/libfoo.dylib\n",
            "p10278\n",
            "ccargo\n",
            "freg\n",
            "n/Users/x/.cargo-target/debug/deps/libserde.rmeta\n",
        );
        assert_eq!(
            holders(stdout),
            vec![
                Holder {
                    pid: 9720,
                    command: "sjel-mcp".to_string(),
                    kind: "txt".to_string(),
                    file: "/Users/x/.cargo-target/release/sjel-mcp".to_string(),
                },
                Holder {
                    pid: 10278,
                    command: "cargo".to_string(),
                    kind: "reg".to_string(),
                    file: "/Users/x/.cargo-target/debug/deps/libserde.rmeta".to_string(),
                },
            ]
        );
    }

    #[test]
    fn a_command_printed_before_its_file_does_not_become_one() {
        // `c` is the command, and it arrives in every block. Without the `first_file` gate
        // it would be read as a holder's file, which is how a parse like this goes wrong.
        let stdout = "p1\ncjust-a-name\n";
        assert!(holders(stdout).is_empty());
    }

    #[test]
    fn nothing_found_parses_to_nothing() {
        assert!(holders("").is_empty());
        // A trailing newline and a level lsof would never write.
        assert!(holders("\n\n").is_empty());
        assert!(holders("xnot-a-tag\n").is_empty());
    }

    #[test]
    fn an_empty_directory_is_free() {
        let dir = crate::testutil::tempdir("inuse-free");
        assert_eq!(usage(&dir), Usage::Free);
    }

    #[test]
    fn a_directory_that_is_not_there_could_not_be_asked_about() {
        let dir = crate::testutil::tempdir("inuse-missing").join("gone");
        match usage(&dir) {
            Usage::Unverifiable(why) => assert!(why.contains("lsof:"), "{why}"),
            other => panic!("a missing directory must not read as free: {other:?}"),
        }
    }

    #[test]
    fn a_process_working_in_the_directory_holds_it() {
        let dir = crate::testutil::tempdir("inuse-held");
        let mut child = Command::new("sleep")
            .arg("30")
            .current_dir(&dir)
            .spawn()
            .expect("sleep");
        // `usage` is a snapshot, so a process that has just been spawned may take a moment
        // to appear. Bounded, so a probe that never answers fails rather than hangs.
        let mut answer = usage(&dir);
        for _ in 0..40 {
            if !matches!(answer, Usage::Free) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            answer = usage(&dir);
        }
        let _ = child.kill();
        let _ = child.wait();
        match answer {
            Usage::Held(holders) => {
                assert!(holders.iter().any(|h| h.pid == child.id()), "{holders:?}");
            }
            other => panic!("expected Held, got {other:?}"),
        }
    }
}
