//! What a session may reach, decided when it starts.
//!
//! A tier fixes two things and nothing else: the plugin set a session is assembled from, and the
//! directory that set is measured against. It is not a setting a client can widen and not a mode a
//! run can change: a grant made inside one tier must not be usable from another, so the only way
//! into another tier is another session. Rationale: `docs/plans/agent.md` §2.
//!
//! The tier does not classify a command yet. Blocks and asks per command are §3's policy
//! classifier, which reads the tier's scope; until it lands, a `project` session's reach is what
//! the `fs` tools already confine it to.

use std::path::{Path, PathBuf};

/// What a session may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Sjel's own data: capability routes and read-only SQL. No shell, no writes.
    Sjel,
    /// One checkout: the coding tools, inside a root under `$HOME`.
    Project,
    /// The user account: the same tools, reaching all of `$HOME`. `sudo` stays blocked.
    Account,
}

/// The tier a session starts in when the client names none.
pub const DEFAULT: Tier = Tier::Project;

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Sjel, Tier::Project, Tier::Account];

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Sjel => "sjel",
            Tier::Project => "project",
            Tier::Account => "account",
        }
    }

    pub fn parse(text: &str) -> Option<Tier> {
        Tier::ALL.into_iter().find(|tier| tier.as_str() == text)
    }

    /// The plugins this tier assembles.
    ///
    /// Every name is one [`crate::plugins::KNOWN`] carries. `rust`, `skills`, `web` and
    /// `subagents` join the coding tiers when §4's port reaches them into this capability.
    pub fn plugins(self) -> Vec<String> {
        let names: &[&str] = match self {
            Tier::Sjel => &["sjel", "sql", "guard", "compaction"],
            Tier::Project | Tier::Account => &["fs", "sjel", "sql", "guard", "compaction"],
        };
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The root this tier accepts, or the reason it refuses the one asked for.
    ///
    /// `requested` is already resolved to a real directory by the caller. `project` measures its
    /// root against `$HOME` and uses it as its own scope; `account` keeps `$HOME` as the scope and
    /// lets the root be any directory inside it.
    pub fn root(self, requested: &Path, home: &Path) -> Result<PathBuf, String> {
        if self == Tier::Sjel {
            return Ok(requested.to_path_buf());
        }
        if !requested.starts_with(home) {
            return Err(format!(
                "a `{}` session reaches only inside {}. {} is outside it",
                self.as_str(),
                home.display(),
                requested.display()
            ));
        }
        Ok(requested.to_path_buf())
    }

    /// The directory a call in this session may touch, given the session's root. `None` is the
    /// tier that reaches no filesystem at all.
    pub fn reach(self, root: &Path, home: &Path) -> Option<PathBuf> {
        match self {
            Tier::Sjel => None,
            Tier::Project => Some(root.to_path_buf()),
            Tier::Account => Some(home.to_path_buf()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn home() -> PathBuf {
        PathBuf::from("/Users/someone")
    }

    #[test]
    fn a_tier_names_itself_and_reads_back() {
        for tier in Tier::ALL {
            assert_eq!(Tier::parse(tier.as_str()), Some(tier));
        }
        assert_eq!(Tier::parse("root"), None);
        assert_eq!(Tier::parse(""), None);
    }

    #[test]
    fn only_the_coding_tiers_carry_a_shell_and_writes() {
        let sjel = Tier::Sjel.plugins();
        assert!(!sjel.iter().any(|name| name == "fs"));
        for tier in [Tier::Project, Tier::Account] {
            let names = tier.plugins();
            assert!(names.iter().any(|name| name == "fs"));
        }
        assert_eq!(Tier::Sjel.reach(Path::new("/tmp"), &home()), None);
    }

    #[test]
    fn every_plugin_a_tier_names_is_one_this_capability_carries() {
        let known: HashSet<&str> = crate::plugins::KNOWN.iter().copied().collect();
        for tier in Tier::ALL {
            for name in tier.plugins() {
                assert!(
                    known.contains(name.as_str()),
                    "`{}` names `{name}`, which is not in KNOWN",
                    tier.as_str()
                );
            }
        }
    }

    #[test]
    fn a_root_outside_the_account_is_refused_with_the_reason() {
        let inside = home().join("Developer/Sjel");
        assert_eq!(Tier::Project.root(&inside, &home()), Ok(inside.clone()));
        assert_eq!(Tier::Account.root(&inside, &home()), Ok(inside));

        let outside = PathBuf::from("/etc");
        let refused = Tier::Project.root(&outside, &home()).unwrap_err();
        assert!(refused.contains("outside"), "{refused}");
        assert!(Tier::Account.root(&outside, &home()).is_err());
    }

    #[test]
    fn a_project_session_reaches_its_root_and_an_account_session_reaches_home() {
        let root = home().join("Developer/Sjel");
        assert_eq!(Tier::Project.reach(&root, &home()), Some(root.clone()));
        assert_eq!(Tier::Account.reach(&root, &home()), Some(home()));
        assert_eq!(Tier::Sjel.reach(&root, &home()), None);
    }
}
