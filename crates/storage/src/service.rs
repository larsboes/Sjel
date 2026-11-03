//! Is a service the policy expects actually running?
//!
//! The report printed `expected_service` verbatim under "Expected running (not findings)" —
//! a claim about the machine that nothing checked. On 2026-10-11 two of the three entries
//! carried a note that contradicted the line above it ("nothing named postgres is running",
//! "not running… Retired"), and the report printed both. This checks the claim the way the
//! report already checks a protected path for existence: the policy declares, the machine
//! answers.
//!
//! Three answers, and the third is why this is not a boolean. An unknown `kind`, or a probe
//! whose own tool is not installed, is `Unverifiable` rather than `Absent`: a policy written
//! for another init system must not read as a stopped service, which is the same rule as
//! "absent thresholds never manufacture an alarm".

use std::process::Command;

/// What the machine answered about one declared service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    Running,
    Absent,
    /// The probe could not be run. The string says why, and it is printed beside the entry.
    Unverifiable(String),
}

impl Presence {
    /// The column the report prints. `ABSENT` is upper case because it is the answer a
    /// reader acts on, and it is the one the report used to give away for free.
    pub fn label(&self) -> &'static str {
        match self {
            Presence::Running => "running",
            Presence::Absent => "ABSENT",
            Presence::Unverifiable(_) => "unverifiable",
        }
    }

    /// The JSON value: a stable lower-case key, never the display label, so a consumer
    /// parses `absent` rather than the report's `ABSENT` column heading.
    pub fn key(&self) -> &'static str {
        match self {
            Presence::Running => "running",
            Presence::Absent => "absent",
            Presence::Unverifiable(_) => "unverifiable",
        }
    }

    /// The JSON answer: `Some(true)`, `Some(false)`, or `None` when the probe did not run.
    pub fn running(&self) -> Option<bool> {
        match self {
            Presence::Running => Some(true),
            Presence::Absent => Some(false),
            Presence::Unverifiable(_) => None,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Presence::Unverifiable(r) => Some(r),
            _ => None,
        }
    }
}

/// The command that answers the question, or `None` when `kind` names no probe this build
/// has. Pure, so the mapping is tested without running anything.
///
/// `uid` is `None` when it could not be read; a launchd label lives in a per-user domain,
/// so without one the answer is "could not ask" rather than "not loaded".
pub fn probe_argv(kind: &str, name: &str, uid: Option<u32>) -> Option<Vec<String>> {
    match kind {
        // `-f` because a process's argv[0] is not always its name: macmon runs as
        // `macmon serve --port 9911`, so an exact-name match would report a live service
        // as absent.
        "process" => Some(vec![
            "pgrep".to_string(),
            "-f".to_string(),
            name.to_string(),
        ]),
        "launchagent" | "launchd" => uid.map(|u| {
            vec![
                "launchctl".to_string(),
                "print".to_string(),
                format!("gui/{u}/{name}"),
            ]
        }),
        "apple-container" | "container" => Some(vec!["container".to_string(), "list".to_string()]),
        _ => None,
    }
}

/// `container list` exits 0 with a header even when nothing is running, so the name is the
/// answer rather than the exit status. Pure for the same reason as `probe_argv`.
pub fn container_lists(stdout: &str, name: &str) -> bool {
    stdout.contains(name)
}

/// Ask the machine. Never panics, never fails a report: an unaskable question is an answer.
pub fn check(kind: &str, name: &str) -> Presence {
    let Some(argv) = probe_argv(kind, name, current_uid()) else {
        return Presence::Unverifiable(format!("no probe for kind `{kind}`"));
    };
    let out = match Command::new(&argv[0]).args(&argv[1..]).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Presence::Unverifiable(format!("no `{}` on PATH", argv[0]));
        }
        Err(e) => return Presence::Unverifiable(e.to_string()),
    };
    if !out.status.success() {
        // pgrep's exit 1, launchctl's "Could not find service", `container`'s own error:
        // all mean the same thing to a reader — nothing answers to this name.
        return Presence::Absent;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if argv[0] == "container" && !container_lists(&text, name) {
        return Presence::Absent;
    }
    Presence::Running
}

/// The current user's uid, or `None` when `id -u` did not answer. Rust's std does not expose
/// `getuid`, and the workspace's `unsafe_code` lint rules out the libc call.
fn current_uid() -> Option<u32> {
    let out = Command::new("id").arg("-u").output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_probe_matches_on_the_full_command_line() {
        // macmon's argv[0] is `macmon serve --port 9911 …`, so an exact-name probe would
        // report a running service as absent.
        assert_eq!(
            probe_argv("process", "macmon", Some(501)),
            Some(vec!["pgrep".into(), "-f".into(), "macmon".into()])
        );
    }

    #[test]
    fn a_launchagent_probe_is_scoped_to_the_users_domain() {
        assert_eq!(
            probe_argv("launchagent", "com.axon.macmon", Some(501)),
            Some(vec![
                "launchctl".into(),
                "print".into(),
                "gui/501/com.axon.macmon".into()
            ])
        );
        assert_eq!(
            probe_argv("launchd", "x", Some(501)).unwrap()[2],
            "gui/501/x"
        );
    }

    #[test]
    fn an_unreadable_uid_leaves_a_launchagent_unverifiable_not_absent() {
        // `None` rather than a wrong domain: a label probed in the wrong domain would read
        // as a stopped service on a machine where it runs.
        assert_eq!(probe_argv("launchagent", "com.axon.macmon", None), None);
    }

    #[test]
    fn an_unknown_kind_has_no_probe_and_is_never_a_verdict() {
        assert_eq!(probe_argv("systemd", "postgres", Some(501)), None);
        assert_eq!(probe_argv("", "x", Some(501)), None);
        assert_eq!(
            check("systemd", "postgres"),
            Presence::Unverifiable("no probe for kind `systemd`".to_string())
        );
    }

    #[test]
    fn a_container_is_present_only_when_its_name_is_listed() {
        assert!(container_lists(
            "NAME  STATE\npostgres  running\n",
            "postgres"
        ));
        assert!(!container_lists("NAME  STATE\n", "postgres"));
    }

    #[test]
    fn the_three_answers_map_to_a_tri_state_and_a_label() {
        assert_eq!(Presence::Running.running(), Some(true));
        assert_eq!(Presence::Absent.running(), Some(false));
        assert_eq!(Presence::Unverifiable("x".into()).running(), None);
        assert_eq!(Presence::Absent.label(), "ABSENT");
        // The JSON key is stable and lower-case; the label is a display choice.
        assert_eq!(Presence::Absent.key(), "absent");
        assert_eq!(Presence::Unverifiable("x".into()).key(), "unverifiable");
        assert_eq!(Presence::Unverifiable("x".into()).reason(), Some("x"));
        assert_eq!(Presence::Running.reason(), None);
    }
}
