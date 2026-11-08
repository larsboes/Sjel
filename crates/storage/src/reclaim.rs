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

    let (target_dir, _) = target::target_dir(root);
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
            args.target,
            args.node_modules,
        );
        outcome.merge(checkout::run(&plan, root, &guard, args.dry_run));
    }

    Ok(outcome)
}
