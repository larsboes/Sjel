//! Which agent harnesses exist, how to tell whether one is installed, and the DeployConfig
//! that drives each — the whole of `tools/lib/harness-registry.ts` plus the four
//! `default*DeployConfig` factories from `tools/packs-*.ts`, both deleted 2026-10-04 when the
//! adapters moved into `src/packs.rs`.
//!
//! Nothing derives a location twice. The overlay comes from `doctor::overlay`, the crate's one
//! resolver (it is also the one that reports its source, which the doctor prints); the state
//! directory follows XDG; every destination keeps the environment override its adapter
//! documented. The values were compared against the TypeScript factories on this Mac before
//! the launcher switched, per the porting recipe in tools/sjel-cli/README.md.
//!
//! One inherited oddity is kept on purpose: the opencode config in TypeScript is a spread of
//! the codex one with five fields overridden, so it inherits codex's `stateEnvVar` and its
//! `validateAdapterFiles`. Both are reproduced rather than tidied, because a port that
//! silently changes which env var an error names is a port that changed behaviour.

use std::path::{Path, PathBuf};

use super::engine::{self, DeployConfig, TreeConvention};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// The adapter copies the skill to a destination it owns; a destination edit is drift.
    Materialized,
    /// The harness reads the Pack source in place through its own settings file.
    Registry,
}

impl Model {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Materialized => "materialized",
            Self::Registry => "registry",
        }
    }
}

pub struct Harness {
    pub id: &'static str,
    pub label: &'static str,
    /// The path whose presence means this harness is installed.
    pub marker: PathBuf,
    pub model: Model,
    /// The CLI that owns deployment for this harness, named in every hint.
    pub cli: &'static str,
    pub config: DeployConfig,
}

pub struct Unsupported {
    pub id: &'static str,
    pub label: &'static str,
    pub why: &'static str,
}

pub struct Registry {
    pub harnesses: Vec<Harness>,
    pub unsupported: Vec<Unsupported>,
}

impl Registry {
    pub fn new(root: &Path) -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let overlay =
            crate::doctor::overlay::resolve_overlay_root(root).map(|o| PathBuf::from(o.root));
        let state_home = std::env::var("XDG_STATE_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("state"));
        let state_dir = state_home.join("axon").join("pack-deployments");

        // Pack roots: the public source of truth first, then the overlay's Packs when this
        // machine has an overlay that carries one.
        let with_overlay = |roots: &mut Vec<PathBuf>| {
            if let Some(overlay) = &overlay {
                let packs = overlay.join("Packs");
                if packs.is_dir() {
                    roots.push(packs);
                }
            }
        };
        let mut claude_roots = vec![root.join("Packs")];
        with_overlay(&mut claude_roots);
        let mut pi_roots = vec![root.join("Packs")];
        with_overlay(&mut pi_roots);
        let mut opencode_roots = vec![root.join("Packs")];
        with_overlay(&mut opencode_roots);

        let env_path = |name: &str| -> Option<PathBuf> {
            std::env::var(name)
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        };
        let resolved = |p: PathBuf| engine::resolve(&p);

        let claude = DeployConfig {
            axon_root: root.to_path_buf(),
            pack_roots: Some(claude_roots),
            destination: resolved(
                env_path("CLAUDE_SKILLS_DIR")
                    .unwrap_or_else(|| home.join(".claude").join("skills")),
            ),
            state_file: resolved(
                env_path("SJEL_CLAUDE_STATE_FILE").unwrap_or_else(|| state_dir.join("claude.json")),
            ),
            adapter: "claude".to_owned(),
            state_env_var: Some("SJEL_CLAUDE_STATE_FILE".to_owned()),
            tree_convention: Some(TreeConvention {
                source_dir: "agents".to_owned(),
                destination_root: resolved(
                    env_path("CLAUDE_AGENTS_DIR")
                        .unwrap_or_else(|| home.join(".claude").join("agents")),
                ),
            }),
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };
        let codex = DeployConfig {
            axon_root: root.to_path_buf(),
            pack_roots: None,
            destination: resolved(
                env_path("CODEX_SKILLS_DIR").unwrap_or_else(|| home.join(".agents").join("skills")),
            ),
            state_file: resolved(
                env_path("SJEL_CODEX_STATE_FILE").unwrap_or_else(|| state_dir.join("codex.json")),
            ),
            adapter: "codex".to_owned(),
            state_env_var: Some("SJEL_CODEX_STATE_FILE".to_owned()),
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: Some(engine::validate_codex_files),
        };
        let opencode = DeployConfig {
            axon_root: root.to_path_buf(),
            pack_roots: Some(opencode_roots),
            destination: resolved(
                env_path("OPENCODE_SKILLS_DIR")
                    .unwrap_or_else(|| home.join(".config").join("opencode").join("skills")),
            ),
            state_file: resolved(
                env_path("SJEL_OPENCODE_PACKS_STATE_FILE")
                    .unwrap_or_else(|| state_dir.join("opencode-packs.json")),
            ),
            adapter: "opencode".to_owned(),
            // Inherited from the codex spread in tools/packs-opencode.ts — see the module note.
            state_env_var: Some("SJEL_CODEX_STATE_FILE".to_owned()),
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: Some(engine::validate_codex_files),
        };
        let pi = DeployConfig {
            axon_root: root.to_path_buf(),
            pack_roots: Some(pi_roots),
            // Unused by pi's skill channel, but getStatuses needs a valid configuration.
            destination: home.join(".pi").join("agent").join("skills"),
            state_file: resolved(
                env_path("SJEL_PI_STATE_FILE").unwrap_or_else(|| state_dir.join("pi.json")),
            ),
            adapter: "pi".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };

        Self {
            harnesses: vec![
                Harness {
                    id: "claude",
                    label: "Claude Code",
                    marker: home.join(".claude"),
                    model: Model::Materialized,
                    cli: "tools/packs-claude",
                    config: claude,
                },
                Harness {
                    id: "codex",
                    label: "Codex",
                    marker: home.join(".codex"),
                    model: Model::Materialized,
                    cli: "tools/packs-codex",
                    config: codex,
                },
                Harness {
                    id: "opencode",
                    label: "opencode",
                    marker: home.join(".config").join("opencode"),
                    model: Model::Materialized,
                    cli: "tools/packs-opencode",
                    config: opencode,
                },
                Harness {
                    id: "pi",
                    label: "pi",
                    marker: home.join(".pi").join("agent").join("settings.json"),
                    model: Model::Registry,
                    cli: "tools/packs-pi",
                    config: pi,
                },
            ],
            unsupported: vec![Unsupported {
                id: "antigravity",
                label: "Antigravity",
                why:
                    "no adapter, and no verified skill/extension format for it in this repository. \
                      Nothing is installed on this machine to measure against (2026-09-07).",
            }],
        }
    }

    pub fn by_id(&self, id: &str) -> Result<&Harness, String> {
        self.harnesses.iter().find(|h| h.id == id).ok_or_else(|| {
            let known: Vec<&str> = self.harnesses.iter().map(|h| h.id).collect();
            format!("unknown harness '{id}'; known: {}", known.join(", "))
        })
    }
}

pub fn is_installed(harness: &Harness) -> bool {
    harness.marker.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_harnesses_and_one_unsupported_row() {
        let registry = Registry::new(Path::new("/tmp/sjel-registry-test"));
        let ids: Vec<&str> = registry.harnesses.iter().map(|h| h.id).collect();
        assert_eq!(ids, vec!["claude", "codex", "opencode", "pi"]);
        assert_eq!(registry.unsupported.len(), 1);
        assert!(registry.by_id("nope").is_err());
    }

    #[test]
    fn the_opencode_config_keeps_the_codex_spread_it_inherits() {
        let registry = Registry::new(Path::new("/tmp/sjel-registry-test"));
        let opencode = registry.by_id("opencode").unwrap();
        assert_eq!(
            opencode.config.state_env_var.as_deref(),
            Some("SJEL_CODEX_STATE_FILE")
        );
        assert!(opencode.config.validate_adapter_files.is_some());
    }
}
