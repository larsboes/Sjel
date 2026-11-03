//! A UTC time in front of every stderr line, so a failure in `/tmp/sjel-<cap>.err` can be matched
//! to what happened at that time.
//!
//! The runner hands a service the log file itself as stderr (`tools/sjel-cli/src/runner.rs`), so
//! the service stamps its own lines: fd 2 becomes a pipe, and one thread copies each line to the
//! file with the time in front. A panic is written to the file directly, because a panic that
//! ends the process would also end the thread before it copied the message.

use std::time::{SystemTime, UNIX_EPOCH};

/// Turns stamping on for this process. Calling it again does nothing. Lines written before the
/// first call keep no time.
pub fn stamp_stderr() {
    #[cfg(unix)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            if let Err(error) = redirect() {
                eprintln!("stderr timestamps unavailable: {error}");
            }
        });
    }
}

#[cfg(unix)]
fn redirect() -> std::io::Result<()> {
    use std::io::Write;
    use std::os::fd::AsFd;

    let file = std::fs::File::from(std::io::stderr().as_fd().try_clone_to_owned()?);
    let panic_file = file.try_clone()?;
    let (reader, writer) = std::io::pipe()?;
    std::thread::Builder::new()
        .name("stderr-stamp".into())
        .spawn(move || copy_lines(reader, file))?;
    rustix::stdio::dup2_stderr(&writer)?;
    drop(writer);
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let backtrace = std::backtrace::Backtrace::capture();
        let mut out = &panic_file;
        let _ = writeln!(out, "{} thread '{name}' {info}", stamp(now_secs()));
        if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            let _ = writeln!(out, "{backtrace}");
        }
    }));
    Ok(())
}

/// Copies until every writer is gone. A failed write is dropped and reading goes on: a full
/// pipe would block every `eprintln!` in the service.
#[cfg(unix)]
fn copy_lines(reader: impl std::io::Read, mut out: std::fs::File) {
    use std::io::{BufRead, Write};

    let mut lines = std::io::BufReader::new(reader);
    let mut line = Vec::new();
    loop {
        line.clear();
        match lines.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let mut stamped = format!("{} ", stamp(now_secs())).into_bytes();
                stamped.extend_from_slice(&line);
                let _ = out.write_all(&stamped);
            }
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `2026-10-25T14:03:07Z`.
fn stamp(secs: i64) -> String {
    let t = secs.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        civil_date::iso_of_unix_day(secs.div_euclid(86_400)),
        t / 3600,
        t / 60 % 60,
        t % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_is_utc_to_the_second() {
        assert_eq!(stamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(stamp(1_793_000_587), "2026-10-26T07:43:07Z");
    }

    #[cfg(unix)]
    #[test]
    fn every_line_gets_a_time_and_a_partial_last_line_is_kept() {
        let path = std::env::temp_dir().join(format!("sjel-stamp-{}", std::process::id()));
        let out = std::fs::File::create(&path).unwrap();
        copy_lines(&b"one\ntwo\nthree"[..], out);
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(
            lines.iter().all(|l| l.len() > 21 && &l[20..21] == " "),
            "{text}"
        );
        assert!(
            lines[0].ends_with("Z one") && lines[2].ends_with("Z three"),
            "{text}"
        );
    }
}
