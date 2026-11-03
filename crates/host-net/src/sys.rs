//! Running a host command and reading its stdout, with one failure shape.
//!
//! Every read in this crate is unprivileged and non-mutating, so the only two things that can
//! go wrong are "the command is not here" and "the command refused". Both must be named rather
//! than turned into an empty result: an exposure report that says nothing because a tool was
//! missing reads exactly like a host with nothing exposed
//! (Packs/harness/skills/sjel/references/shared-failure-policy.md).

use std::process::Command;

/// Run `cmd` with `args` and return its stdout.
///
/// `Err` carries a one-line reason for the operator. Standard error is folded in when the
/// command ran and refused, because that text is usually the whole diagnosis.
pub fn capture(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().next().unwrap_or("").trim();
        let code = out
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into());
        return Err(if first.is_empty() {
            format!("{cmd} exited {code}")
        } else {
            format!("{cmd} exited {code}: {first}")
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
