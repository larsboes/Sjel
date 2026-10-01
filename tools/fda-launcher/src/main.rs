//! `sjel-fda-launcher <program> [args...]`: run the program as a child, wait, and exit
//! with its status.
//!
//! macOS decides whether a process may read another app's iCloud container, such as
//! Obsidian's vault, by asking which process is *responsible* for it. A launchd job's
//! responsible process is the executable launchd started. For a scheduled Axon job that
//! was `/bin/bash`, which has no Full Disk Access, so `rsync` got "Operation not
//! permitted" on the vault and `ls` could not list the iCloud Drive backup folder
//! (measured 2026-09-25, `tools/backup.sh` traced under `launchctl submit`).
//!
//! Granting Full Disk Access to `/bin/bash` would grant it to every script on the Mac.
//! This binary is the narrow alternative: it is granted once, and the children it spawns
//! inherit its responsibility. It must spawn and wait rather than `exec`: after an exec
//! the process is bash again, and so is its identity.
//!
//! The grant is keyed on the code signature, so the binary is signed and installed
//! outside `target/` by `tools/fda-launcher/install`, where a rebuild cannot replace it.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(program) = args.next() else {
        eprintln!("usage: sjel-fda-launcher <program> [args...]");
        return ExitCode::from(64);
    };
    match Command::new(&program).args(args).status() {
        Ok(status) => ExitCode::from(exit_byte(status)),
        Err(error) => {
            eprintln!(
                "sjel-fda-launcher: cannot start {}: {error}",
                program.to_string_lossy()
            );
            ExitCode::from(127)
        }
    }
}

/// The child's exit code, or 128 + the signal that ended it, as a shell reports it.
fn exit_byte(status: std::process::ExitStatus) -> u8 {
    if let Some(code) = status.code() {
        return u8::try_from(code).unwrap_or(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return u8::try_from(128 + signal).unwrap_or(1);
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_child_exit_code_passes_through() {
        let status = Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .status()
            .unwrap();
        assert_eq!(exit_byte(status), 3);
        let status = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .status()
            .unwrap();
        assert_eq!(exit_byte(status), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_signalled_child_reports_128_plus_the_signal() {
        let status = Command::new("/bin/sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .unwrap();
        assert_eq!(exit_byte(status), 128 + 15);
    }
}
