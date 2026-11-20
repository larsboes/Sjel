//! Policy and checkout reclaim through one path guard.

use std::path::Path;

use crate::checkout::{self, Plan};
use crate::classes;
use crate::guard::{Authority, Guard};
use crate::policy::Policy;
use crate::target;

/// What a run did, or would do. One type for both halves, so the verb totals them without
/// translating between two spellings of the same three numbers.
#[derive(Debug, Default)]
pub struct Outcome {
    pub lines: Vec<String>,
    /// The guard said no. Exit 1.
    pub refused: usize,
    /// A gate held it back, which is the policy working. Not an error.
    pub skipped: usize,
    /// Bytes the run gives back — the plan minus every refusal.
    pub bytes: u64,
    /// What `bytes` leaves out because a larger surviving candidate covers it in the same plan.
    pub nested_bytes: u64,
}

impl Outcome {
    /// Fold another half's result in, keeping the order the halves ran in.
    fn merge(&mut self, other: Outcome) {
        self.lines.extend(other.lines);
        self.refused += other.refused;
        self.skipped += other.skipped;
        self.bytes += other.bytes;
        self.nested_bytes += other.nested_bytes;
    }
}

/// What the caller asked for.
#[derive(Debug, Default, Clone)]
pub struct Args {
    pub dry_run: bool,
    pub force: bool,
    /// Act only on classes the policy itself bounds, by cap or by age.
    pub scheduled: bool,
    /// Restrict the policy half to one class by name.
    pub only: Option<String>,
    /// Act on the checkout's build output instead of the policy's classes. Replaces the
    /// class scope rather than adding to it: `reclaim --repo` quietly also clearing the app
    /// caches is not something to discover after the fact.
    pub repo: bool,
    pub incremental: bool,
    pub profile: Option<String>,
    pub checkout_target: bool,
    pub target: bool,
    pub node_modules: bool,
}

impl Args {
    fn wants_classes(&self) -> bool {
        !self.repo
    }

    fn wants_repo(&self) -> bool {
        self.repo
    }
}

/// Parse profile selectors without silently falling back to unscoped reclaim.
pub fn profile_options(args: &[String]) -> Result<(Option<String>, bool), String> {
    let mut profile = None;
    let mut checkout_target = false;
    for (i, arg) in args.iter().enumerate() {
        if arg.starts_with("--checkout-target=") {
            return Err("--checkout-target does not take a value".into());
        }
        if arg == "--checkout-target" {
            checkout_target = true;
        }
        let value = if arg == "--profile" {
            args.get(i + 1)
                .ok_or("--profile needs a Cargo profile name")?
                .as_str()
        } else if let Some(value) = arg.strip_prefix("--profile=") {
            value
        } else {
            continue;
        };
        if profile.is_some() {
            return Err("--profile may be specified only once".into());
        }
        if !valid_profile(value) {
            return Err("--profile needs a Cargo profile name, not a flag or path".into());
        }
        profile = Some(value.to_owned());
    }
    Ok((profile, checkout_target))
}

fn valid_profile(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Load the policy, or `None` when the machine has no overlay.
///
/// `prune`'s whole origin is working on a clone with no overlay, which is when someone most
/// needs the disk back, so a scope that does not read the policy must not fail on a missing one.
pub fn load_policy() -> Option<Policy> {
    let path = crate::policy_path().ok()?;
    crate::policy::load(&path).ok()
}

/// Act. Returns the outcome, or the message a usage error prints.
pub fn run(
    policy: Option<&Policy>,
    home: &str,
    root: &Path,
    args: &Args,
) -> Result<Outcome, String> {
    let mut outcome = Outcome::default();

    if !args.wants_classes() && !args.wants_repo() {
        return Err(
            "reclaim needs something to act on: the policy's classes, or --repo with one of \
             --incremental, --target, --node-modules"
                .to_string(),
        );
    }

    if args.repo && args.only.is_some() {
        return Err(
            "`--only` names a policy class, and `--repo` acts on the checkout's build output \
             instead — they cannot be asked for together"
                .to_string(),
        );
    }

    if args.checkout_target && args.profile.is_none() {
        return Err("--checkout-target requires --profile NAME".into());
    }
    if let Some(profile) = args.profile.as_deref() {
        if !valid_profile(profile) {
            return Err("--profile needs a Cargo profile name, not a flag or path".into());
        }
        if !args.repo || !args.incremental || args.target || args.node_modules || args.scheduled {
            return Err(
                "--profile requires --repo --incremental without other reclaim scopes".into(),
            );
        }
    }
    let target_dir = if args.checkout_target {
        let path = root.join("target");
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_dir() => {
                return Err(format!("{} is not a real directory", path.display()));
            }
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {err}", path.display()));
            }
            _ => {}
        }
        path
    } else {
        target::target_dir(root).0
    };
    // An empty authority is a machine with no policy, and every guard that depends on the
    // policy then has nothing to say — which is the behaviour `prune` has always had.
    let authority = match policy {
        Some(policy) => Authority::from_policy(policy, home),
        None => Authority::default(),
    };
    let guard = Guard::new(&authority, root, &target_dir, args.force);

    if args.wants_classes() {
        let policy = policy.ok_or_else(|| {
            "no overlay — the policy's classes are in <overlay>/config/storage-policy.toml. \
             Use `reclaim --repo ...` on a clone with no overlay."
                .to_string()
        })?;
        // `--only` is a filter over the class list, so it needs nothing further.
        if args
            .only
            .as_ref()
            .is_some_and(|name| !policy.classes.iter().any(|class| &class.name == name))
        {
            return Err(format!(
                "unknown storage class: {}",
                args.only.as_deref().unwrap_or_default()
            ));
        }
        let measured = classes::measure_selected(policy, home, args.only.as_deref());
        outcome.merge(classes::run(
            &measured,
            &guard,
            args.scheduled,
            args.only.as_deref(),
            args.dry_run,
        ));
    }

    if args.wants_repo() {
        if !(args.incremental || args.target || args.node_modules) {
            return Err(
                "reclaim --repo needs at least one of --incremental, --target, --node-modules: \
                 deleting every node_modules in a checkout is not something to discover after \
                 the fact"
                    .to_string(),
            );
        }
        let plan = Plan::build(
            root,
            &target_dir,
            args.incremental,
            args.profile.as_deref(),
            args.target,
            args.node_modules,
        );
        outcome.merge(checkout::run(&plan, root, &guard, args.dry_run));
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_options_reject_missing_invalid_or_repeated_names() {
        let parse = |args: &[&str]| {
            profile_options(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(
            parse(&["--profile=desktop-dev", "--checkout-target"]).unwrap(),
            (Some("desktop-dev".into()), true)
        );
        for args in [
            &["--profile"][..],
            &["--profile", "--dry-run"],
            &["--profile=../release"],
            &["--profile", "debug", "--profile=release"],
            &["--checkout-target=false"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn explicit_checkout_target_does_not_select_the_inherited_cache() {
        let repo = crate::testutil::tempdir("checkout-profile");
        let incremental = repo.join("target/debug/incremental");
        std::fs::create_dir_all(&incremental).unwrap();
        let args = Args {
            dry_run: true,
            repo: true,
            incremental: true,
            profile: Some("debug".into()),
            checkout_target: true,
            ..Default::default()
        };
        let outcome = run(None, "", &repo, &args).unwrap();
        assert_eq!(outcome.refused, 0, "{:?}", outcome.lines);
        assert!(outcome.lines[0].contains("target/debug/incremental"));
        assert!(incremental.exists());

        let incompatible = Args {
            target: true,
            ..args
        };
        assert!(run(None, "", &repo, &incompatible).is_err());
    }
}
