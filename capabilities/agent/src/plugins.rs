//! The plugin set one run carries.
//!
//! A plugin is an [`Extension`] (the contract in `libs/agent/src/extension.rs`), except `fs`,
//! which is the core coding tool set and therefore has no hooks of its own. The set is named by
//! `<overlay>/config/agent.toml` under `[agent] extensions`, the same key the CLI reads, so the
//! app and the terminal cannot disagree about what an agent may do.
//!
//! The approval gate is not in the list. It is appended to every set, because a plugin list that
//! could switch it off would be a plugin list that can make the agent unaccountable.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use sjel_agent::{ext, Extension, Tool};

use crate::gate::ApprovalGate;
use crate::session::Grants;
use crate::sql::Sql;

/// What a run carries when the overlay names nothing.
pub const DEFAULT: [&str; 6] = ["fs", "graph", "sjel", "sql", "guard", "compaction"];

/// Every name this binary can assemble.
pub const KNOWN: [&str; 6] = ["fs", "graph", "sjel", "sql", "guard", "compaction"];

const DEFAULT_SHELL_URL: &str = "http://127.0.0.1:8082";

/// One run's tools and extensions, ready for `Agent::new`.
pub struct Set {
    pub tools: Vec<Box<dyn Tool>>,
    pub extensions: Vec<Box<dyn Extension>>,
}

/// Builds the set. `grants` is the session's grant list, shared with the HTTP handler that records
/// an Allow, so an approval made in the app applies to the call that is waiting for it.
pub fn assemble(
    names: &[String],
    root: &Path,
    reach: Option<std::path::PathBuf>,
    stop: &Arc<AtomicBool>,
    log: Arc<crate::session::EventLog>,
    grants: Arc<Grants>,
    max_input_tokens: Option<u32>,
) -> Result<Set, String> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut extensions: Vec<Box<dyn Extension>> = Vec::new();
    let mut read_only: HashSet<String> = HashSet::new();

    for name in names {
        match name.as_str() {
            "fs" => {
                let coding = sjel_agent::tools::coding(root, stop);
                read_only.extend(
                    coding
                        .iter()
                        .filter(|tool| tool.read_only())
                        .map(|tool| tool.name().to_owned()),
                );
                tools.extend(coding);
            }
            "graph" => {
                let graph = ext::graph::Graph::from_env();
                read_only.extend(graph.tools().iter().map(|tool| tool.name().to_owned()));
                extensions.push(Box::new(graph));
            }
            "sjel" => {
                let sjel = ext::sjel::SjelTools::new(
                    std::env::var("SJEL_STATUS_URL")
                        .unwrap_or_else(|_| DEFAULT_SHELL_URL.to_owned()),
                    sjel_server::InboundAuth::from_deployment().bearer_header(),
                );
                read_only.extend(sjel.tools().iter().map(|tool| tool.name().to_owned()));
                extensions.push(Box::new(sjel));
            }
            "sql" => {
                let sql = Sql::new();
                read_only.extend(sql.tools().iter().map(|tool| tool.name().to_owned()));
                extensions.push(Box::new(sql));
            }
            "guard" => extensions.push(Box::new(ext::guard::Guard::new(stop))),
            "compaction" => {
                extensions.push(Box::new(ext::compaction::Compaction::new(max_input_tokens)))
            }
            other => {
                return Err(format!(
                    "unknown plugin `{other}`. This capability carries: {}",
                    KNOWN.join(", ")
                ))
            }
        }
    }

    // The guard is named by config; the approval gate is not.
    extensions.push(Box::new(ApprovalGate::new(
        log,
        root.to_path_buf(),
        reach,
        read_only,
        grants,
        Arc::clone(stop),
    )));

    Ok(Set { tools, extensions })
}

/// The plugin names from `<overlay>/config/agent.toml`, or [`DEFAULT`].
///
/// An unreadable or malformed file is reported and the defaults are used: a typo in a config file
/// must not leave a session with no tools at all.
pub fn configured() -> Vec<String> {
    let Some(path) = sjel_config::overlay_config("agent.toml") else {
        return DEFAULT.iter().map(|name| (*name).to_owned()).collect();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return DEFAULT.iter().map(|name| (*name).to_owned()).collect();
    };
    match parse_extensions(&text) {
        Ok(names) if !names.is_empty() => names,
        Ok(_) => DEFAULT.iter().map(|name| (*name).to_owned()).collect(),
        Err(reason) => {
            eprintln!(
                "{}: {reason} — using the default plugin set",
                path.display()
            );
            DEFAULT.iter().map(|name| (*name).to_owned()).collect()
        }
    }
}

/// `[agent] extensions = ["fs", "graph"]`, and nothing else in the file.
fn parse_extensions(text: &str) -> Result<Vec<String>, String> {
    // `toml::from_str`, not `parse`: in toml 1.x a `Value`'s `FromStr` reads one value, and a
    // document starting `[agent]` is a table header rather than an array.
    let value: toml::Value = toml::from_str(text).map_err(|e| format!("{e}"))?;
    let Some(list) = value
        .get("agent")
        .and_then(|agent| agent.get("extensions"))
        .and_then(toml::Value::as_array)
    else {
        return Ok(Vec::new());
    };
    list.iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "every entry under `[agent] extensions` must be a string".to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sjel-agent-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_config_file_names_the_set_the_cli_names() {
        let names = parse_extensions("[agent]\nextensions = [\"fs\", \"graph\"]\n").unwrap();
        assert_eq!(names, ["fs", "graph"]);
        assert!(parse_extensions("[agent]\n").unwrap().is_empty());
        assert!(parse_extensions("").unwrap().is_empty());
        assert!(parse_extensions("[agent]\nextensions = [1]\n").is_err());
        assert!(parse_extensions("not toml at all [").is_err());
    }

    #[test]
    fn every_plugin_assembles_and_the_gate_is_always_last() {
        let dir = scratch("assemble");
        let stop = Arc::new(AtomicBool::new(false));
        for name in KNOWN {
            let names = vec![name.to_owned()];
            let set = assemble(
                &names,
                &dir,
                Some(dir.clone()),
                &stop,
                Arc::new(crate::session::EventLog::new()),
                Grants::new(),
                None,
            )
            .unwrap_or_else(|reason| panic!("`{name}` did not assemble: {reason}"));
            assert_eq!(
                set.extensions.last().map(|e| e.name()),
                Some(crate::gate::NAME),
                "`{name}` did not end with the approval gate"
            );
        }
    }

    #[test]
    fn an_unknown_plugin_names_what_exists() {
        let dir = scratch("unknown");
        let stop = Arc::new(AtomicBool::new(false));
        let err = match assemble(
            &["typo".to_owned()],
            &dir,
            Some(dir.clone()),
            &stop,
            Arc::new(crate::session::EventLog::new()),
            Grants::new(),
            None,
        ) {
            Ok(_) => panic!("an unknown plugin assembled"),
            Err(reason) => reason,
        };
        assert!(err.contains("unknown plugin `typo`"), "{err}");
        for name in KNOWN {
            assert!(err.contains(name), "`{name}` is not named: {err}");
        }
    }

    #[test]
    fn the_read_only_set_is_what_the_gate_will_not_ask_about() {
        let dir = scratch("read-only");
        let stop = Arc::new(AtomicBool::new(false));
        let set = assemble(
            &["fs".to_owned()],
            &dir,
            Some(dir.clone()),
            &stop,
            Arc::new(crate::session::EventLog::new()),
            Grants::new(),
            None,
        )
        .unwrap();
        let names: Vec<&str> = set.tools.iter().map(|t| t.name()).collect();
        for read_only in ["read", "grep", "find", "ls"] {
            assert!(
                names.contains(&read_only),
                "{read_only} missing from {names:?}"
            );
        }
        for writing in ["write", "edit", "bash"] {
            assert!(names.contains(&writing), "{writing} missing from {names:?}");
        }
    }
}
