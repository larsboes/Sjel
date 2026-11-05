//! What a session may reach, decided when it starts.

pub use agent_api::{Tier, DEFAULT_TIER as DEFAULT};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

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
