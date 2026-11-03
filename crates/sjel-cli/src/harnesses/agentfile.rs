//! `tools/lib/pi-agent-file.ts` — translate a Claude-Code-native agent file into the shape
//! pi-subagents can load. Ported 2026-10-04.
//!
//! Two things differ, and both are hard errors rather than a passthrough because the failure
//! they prevent is silent:
//!
//! - `tools:` names differ in case, and `Glob` does not exist in pi at all (it searches with
//!   `find`). pi-subagents drops an unrecognised name from the agent's allowlist with only a
//!   `tools-error` event, so a copied council member would come up with no read tool.
//! - `model:` names a Claude Code tier (`opus`, `sonnet`, `haiku`). On pi a tier becomes the
//!   `provider/id` that `[tiers]` in the overlay's `config/pi-models.toml` maps it to, so a small
//!   session model can hand review work to a stronger one. A tier with no mapping is dropped, and
//!   the agent inherits the session model.
//!
//! The body is returned byte-for-byte; only the frontmatter is touched.

use std::collections::BTreeMap;
use std::path::Path;

/// Claude Code tool name → pi builtin. Covers the full pi builtin set (7 tools).
const PI_TOOL_NAME: &[(&str, &str)] = &[
    ("Read", "read"),
    ("Write", "write"),
    ("Edit", "edit"),
    ("Bash", "bash"),
    ("Grep", "grep"),
    ("Glob", "find"),
    ("LS", "ls"),
];

/// The builtin names pi actually registers: `createCodingTools` → read/bash/edit/write,
/// `createReadOnlyTools` → grep/find/ls.
const PI_BUILTIN_TOOL_NAMES: &[&str] = &["read", "bash", "edit", "write", "grep", "find", "ls"];

/// Values that are selectors rather than tool names, and pass through untouched.
fn is_selector(name: &str) -> bool {
    matches!(name, "*" | "all" | "none")
}

fn translate_tool_list(value: &str, label: &str) -> Result<String, String> {
    let names: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() {
        return Err(format!("{label}: 'tools:' is present but empty"));
    }
    let mut translated = Vec::with_capacity(names.len());
    for name in names {
        if is_selector(&name.to_lowercase()) {
            translated.push(name.to_lowercase());
            continue;
        }
        if name.starts_with("ext:") {
            translated.push(name.to_owned());
            continue;
        }
        let Some(mapped) = PI_TOOL_NAME
            .iter()
            .find(|(from, _)| *from == name)
            .map(|(_, to)| *to)
        else {
            let known = PI_TOOL_NAME
                .iter()
                .map(|(from, _)| *from)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "{label}: tool '{name}' has no pi equivalent. Claude Code names are TitleCase and \
                 pi's are lowercase; if this is a new tool, add it to PI_TOOL_NAME in \
                 tools/sjel-cli/src/harnesses/agentfile.rs. Known: {known}"
            ));
        };
        if !PI_BUILTIN_TOOL_NAMES.contains(&mapped) {
            return Err(format!(
                "{label}: '{name}' maps to '{mapped}', which is not a pi builtin"
            ));
        }
        translated.push(mapped.to_owned());
    }
    Ok(translated.join(", "))
}

/// The overlay's tier map, `[tiers]` in `config/pi-models.toml`. Empty when the overlay or the
/// file is absent, so every pin is dropped as before.
pub fn pi_model_tiers(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let Some(overlay) = crate::doctor::overlay::resolve_overlay_root(root) else {
        return Ok(BTreeMap::new());
    };
    let path = Path::new(&overlay.root)
        .join("config")
        .join("pi-models.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(BTreeMap::new());
    };
    parse_tiers(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse_tiers(text: &str) -> Result<BTreeMap<String, String>, String> {
    let value: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
    let Some(tiers) = value.get("tiers") else {
        return Ok(BTreeMap::new());
    };
    let table = tiers.as_table().ok_or("`tiers` must be a table")?;
    table
        .iter()
        .map(|(tier, model)| {
            let model = model
                .as_str()
                .filter(|m| m.contains('/'))
                .ok_or_else(|| format!("tiers.{tier} must be a \"provider/id\" string"))?;
            Ok((tier.clone(), model.to_owned()))
        })
        .collect()
}

/// Rewrite one Claude-Code-native agent file for pi, mapping its tier through the overlay.
pub fn translate_agent_for_pi(content: &str, label: &str, root: &Path) -> Result<String, String> {
    translate_with_tiers(content, label, &pi_model_tiers(root)?)
}

fn translate_with_tiers(
    content: &str,
    label: &str,
    tiers: &BTreeMap<String, String>,
) -> Result<String, String> {
    let text = content.strip_prefix('\u{feff}').unwrap_or(content);
    if !text.starts_with("---") {
        return Err(format!(
            "{label}: no frontmatter block, refusing to translate"
        ));
    }
    let Some(end) = text[3..].find("\n---").map(|i| i + 3) else {
        return Err(format!("{label}: unterminated frontmatter block"));
    };

    let head = &text[4..end];
    let tail = &text[end..];

    let mut saw_tools = false;
    let mut pin: Option<(String, Option<String>)> = None;
    let mut rewritten = Vec::new();
    for line in head.split('\n') {
        if let Some(value) = line.strip_prefix("tools:") {
            saw_tools = true;
            rewritten.push(format!(
                "tools: {}",
                translate_tool_list(value.trim_start_matches([' ', '\t']), label)?
            ));
            continue;
        }
        if let Some(value) = line.strip_prefix("model:") {
            let tier = value.trim().to_owned();
            let mapped = tiers.get(&tier).cloned();
            if let Some(model) = &mapped {
                rewritten.push(format!("model: {model}"));
            }
            pin = Some((tier, mapped));
            continue;
        }
        rewritten.push(line.to_owned());
    }
    if !saw_tools {
        return Err(format!(
            "{label}: no 'tools:' line. pi-subagents defaults to all seven builtins, which would \
             give this agent write and edit access. Declare the read-only set explicitly."
        ));
    }

    let provenance = [
        format!("# Generated by Sjel tools/sjel-cli from {label} — do not edit this copy;"),
        "# edit the Pack and re-run. Tool names are remapped for pi (Glob -> find).".to_owned(),
        match &pin {
            Some((tier, Some(model))) => format!(
                "# The Claude Code '{tier}' pin maps to {model} through the overlay's config/pi-models.toml."
            ),
            Some(_) => "# The Claude Code 'model:' pin was removed: on pi this agent inherits the session model."
                .to_owned(),
            None => "# This agent carried no 'model:' pin.".to_owned(),
        },
    ]
    .join("\n");

    Ok(format!("---\n{provenance}\n{}{tail}", rewritten.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "---\nname: Explore\ndescription: 'search'\ntools: Read, Bash, Grep, Glob, LS\nmodel: sonnet\n---\n\n# Body\n\nText here.\n";

    #[test]
    fn tool_names_are_remapped_and_the_model_pin_is_dropped() {
        let out = translate_with_tiers(CLAUDE, "Packs/harness/agents/Explore.md", &BTreeMap::new())
            .unwrap();
        assert!(out.contains("tools: read, bash, grep, find, ls"), "{out}");
        // The provenance comment names `model:`, so the check is for a frontmatter LINE.
        assert!(!out.lines().any(|l| l.starts_with("model:")), "{out}");
        assert!(
            out.contains("# The Claude Code 'model:' pin was removed"),
            "{out}"
        );
        // The body is byte-for-byte, including the leading blank line after the frontmatter.
        assert!(out.ends_with("\n\n# Body\n\nText here.\n"), "{out}");
    }

    #[test]
    fn a_mapped_tier_becomes_the_pi_model_and_an_unmapped_one_is_dropped() {
        let tiers = parse_tiers("[tiers]\nopus = \"openai/gpt-6.1-sol\"\n").unwrap();
        let opus = CLAUDE.replace("model: sonnet", "model: opus");
        let out = translate_with_tiers(&opus, "x", &tiers).unwrap();
        assert!(
            out.lines().any(|l| l == "model: openai/gpt-6.1-sol"),
            "{out}"
        );
        assert!(
            out.contains("'opus' pin maps to openai/gpt-6.1-sol"),
            "{out}"
        );
        let out = translate_with_tiers(CLAUDE, "x", &tiers).unwrap();
        assert!(!out.lines().any(|l| l.starts_with("model:")), "{out}");
    }

    #[test]
    fn a_tier_without_a_provider_is_refused() {
        assert!(parse_tiers("[tiers]\nopus = \"sol\"\n").is_err());
        assert!(parse_tiers("").unwrap().is_empty());
    }

    #[test]
    fn selectors_and_ext_tools_pass_through() {
        let file = "---\ntools: *\n---\nbody\n";
        assert!(translate_with_tiers(file, "x", &BTreeMap::new())
            .unwrap()
            .contains("tools: *"));
        let file = "---\ntools: ALL\n---\nbody\n";
        assert!(translate_with_tiers(file, "x", &BTreeMap::new())
            .unwrap()
            .contains("tools: all"));
        let file = "---\ntools: ext:council\n---\nbody\n";
        assert!(translate_with_tiers(file, "x", &BTreeMap::new())
            .unwrap()
            .contains("tools: ext:council"));
    }

    #[test]
    fn an_unknown_tool_is_refused_rather_than_degraded() {
        let file = "---\ntools: Read, Telepathy\n---\nbody\n";
        let err = translate_with_tiers(file, "x", &BTreeMap::new()).unwrap_err();
        assert!(err.contains("no pi equivalent"), "{err}");
    }

    #[test]
    fn a_missing_tools_line_is_refused_because_the_default_is_all_seven() {
        let file = "---\nname: a\ndescription: d\n---\nbody\n";
        let err = translate_with_tiers(file, "x", &BTreeMap::new()).unwrap_err();
        assert!(err.contains("no 'tools:' line"), "{err}");
    }

    #[test]
    fn no_frontmatter_or_an_unterminated_one_is_refused() {
        assert!(translate_with_tiers("body only", "x", &BTreeMap::new()).is_err());
        assert!(translate_with_tiers("---\ntools: Read\nbody\n", "x", &BTreeMap::new()).is_err());
    }
}
