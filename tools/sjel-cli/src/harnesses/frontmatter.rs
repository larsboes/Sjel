//! The two SKILL.md frontmatter keys the Pack engine reads, without a YAML crate.
//!
//! tools/lib/pack-deploy.ts parsed frontmatter with `Bun.YAML.parse` and then read
//! exactly two keys: `name`, which must equal the unit key, and `description`, which must be
//! a non-empty string of at most 1024 characters. Everything else in the block is ignored.
//! That is the whole contract, so this reads the block directly instead of pulling a YAML
//! dependency into the workspace for two scalars.
//!
//! The precedent is `Packs/writing/skills/skill-creator/scripts/validate_metadata.py`, which
//! parses the same block the same way: a top-level `key:`, a folded or literal block scalar,
//! or a quoted inline scalar. Where it and this differ from real YAML is named rather than
//! left to be discovered: a nested mapping is not descended into, so a `name:` under another
//! key does not count, and an inline ` #` comment is cut the way YAML cuts it. Of the 47
//! SKILL.md files in this repository (2026-10-02), three use `>-` folded descriptions and
//! none use quotes or inline comments on `name:`/`description:`.

/// The text between the opening and closing `---` delimiters, when the file has one.
///
/// The opening delimiter is `---` plus optional whitespace to the end of the line, and the
/// terminator is the first following line that is exactly `---` — the same first-match rule
/// `^---\s*\n([\s\S]*?)\n---(?:\r?\n|$)` states.
pub fn block(text: &str) -> Option<&str> {
    let after_open = text.strip_prefix("---")?;
    let open_end = after_open.find('\n')?;
    if !after_open[..open_end].chars().all(char::is_whitespace) {
        return None;
    }
    let body_start = open_end + 1;
    let mut idx = body_start;
    loop {
        let end = text[idx..].find('\n').map_or(text.len(), |p| idx + p);
        if text[idx..end].trim_end_matches('\r') == "---" {
            return Some(&text[body_start..idx]);
        }
        if end == text.len() {
            return None;
        }
        idx = end + 1;
    }
}

/// Whether the block is a YAML mapping at all: at least one top-level `key:` line.
///
/// A scalar block (`just a string`) and a sequence (`- one`) are both non-mappings, which is
/// the check `extractFrontmatter` makes when it refuses anything that is not an object.
pub fn is_mapping(block: &str) -> bool {
    block.lines().any(is_top_level_key_line)
}

fn is_top_level_key_line(line: &str) -> bool {
    let line = line.trim_end_matches('\r');
    if line.is_empty() || line.starts_with([' ', '\t']) || line.starts_with('#') {
        return false;
    }
    // A key is one word followed by a colon and then end-of-line or whitespace — never
    // `http://x`, where the colon sits inside a bare scalar with no key at all.
    let Some(i) = line.find(':') else {
        return false;
    };
    let key = &line[..i];
    if key.is_empty() || key.contains(char::is_whitespace) {
        return false;
    }
    let rest = &line[i + 1..];
    rest.is_empty() || rest.starts_with([' ', '\t'])
}

/// One top-level scalar, with quotes removed and a block scalar folded.
///
/// `None` when the key is absent, which is what the engine's `name`/`description` checks
/// treat as a missing or wrong-typed value.
pub fn scalar(block: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    let lines: Vec<&str> = block.lines().collect();
    let (i, line) = lines
        .iter()
        .enumerate()
        .find(|(_, l)| l.starts_with(&prefix) && !l.starts_with([' ', '\t']))?;
    let rest = line[prefix.len()..].trim();

    // A block scalar folds every following indented or blank line, stopping at the first
    // line that is neither. `|` and `>` differ in what they do with the breaks; for a
    // non-emptiness and 1024-character check both fold to one line, as the Python
    // validator's parser does.
    if rest.starts_with('|') || rest.starts_with('>') {
        let mut folded: Vec<&str> = Vec::new();
        for cont in &lines[i + 1..] {
            if cont.trim().is_empty() {
                continue;
            }
            if cont.starts_with([' ', '\t']) {
                folded.push(cont.trim());
                continue;
            }
            break;
        }
        return Some(folded.join(" "));
    }

    // A quoted scalar is taken whole — a `#` inside it is part of the value — and a plain one
    // loses a ` #` comment the way YAML drops it.
    let unquoted = if rest.starts_with(['"', '\'']) {
        strip_quotes(rest)
    } else {
        cut_comment(rest).trim()
    };
    Some(unquoted.to_owned())
}

fn strip_quotes(value: &str) -> &str {
    for q in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(q) && value.ends_with(q) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// YAML drops a ` #` comment from a plain scalar. Called before the quotes are stripped, so a
/// `#` inside a quoted value is part of the value.
fn cut_comment(value: &str) -> &str {
    match value.find(" #") {
        Some(i) => &value[..i],
        None => value,
    }
}

/// The engine's `validateUnit` for a skill unit, in the order it checks.
///
/// Errors carry the same text pack-deploy.ts produces, because they are reported verbatim as
/// a StatusRow's `detail` and printed by `tools/harnesses` and `tools/doctor`.
pub fn validate_skill(skill_md: &str, key: &str, label: &str) -> Result<(), String> {
    let Some(block) = block(skill_md) else {
        return Err(format!("{label}: SKILL.md has no valid YAML frontmatter"));
    };
    if !is_mapping(block) {
        return Err(format!("{label}: SKILL.md frontmatter must be a mapping"));
    }
    if scalar(block, "name").as_deref() != Some(key) {
        return Err(format!("{label}: SKILL.md name must be '{key}'"));
    }
    let description = scalar(block, "description").unwrap_or_default();
    if description.trim().is_empty() {
        return Err(format!(
            "{label}: SKILL.md description must be a non-empty string"
        ));
    }
    if description.chars().count() > 1024 {
        return Err(format!(
            "{label}: SKILL.md description exceeds 1024 characters"
        ));
    }
    Ok(())
}

/// The codex adapter's extra check: `agents/openai.yaml`, when a Pack carries one, must be a
/// mapping. `validateCodexFiles` parses it with Bun.YAML and refuses anything else.
pub fn validate_openai_yaml(text: &str, label: &str) -> Result<(), String> {
    if is_mapping(text) {
        return Ok(());
    }
    Err(format!(
        "{label}: invalid agents/openai.yaml: top level must be a mapping"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_inline_scalar() {
        let text = "---\nname: trim\ndescription: does a thing\n---\n\nbody\n";
        assert_eq!(
            scalar(block(text).unwrap(), "name").as_deref(),
            Some("trim")
        );
        assert_eq!(
            scalar(block(text).unwrap(), "description").as_deref(),
            Some("does a thing")
        );
    }

    #[test]
    fn folds_a_block_scalar() {
        let text = "---\nname: x\ndescription: >-\n  one line\n  and another\n\n  a third\nallowed-tools: Read\n---\n";
        assert_eq!(
            scalar(block(text).unwrap(), "description").as_deref(),
            Some("one line and another a third")
        );
        assert_eq!(scalar(block(text).unwrap(), "name").as_deref(), Some("x"));
    }

    #[test]
    fn a_quoted_value_keeps_a_hash_that_is_inside_it() {
        let text = "---\nname: x\ndescription: \"a # b\"\n---\n";
        assert_eq!(
            scalar(block(text).unwrap(), "description").as_deref(),
            Some("a # b")
        );
        let commented = "---\nname: x\ndescription: plain # note\n---\n";
        assert_eq!(
            scalar(block(commented).unwrap(), "description").as_deref(),
            Some("plain")
        );
    }

    #[test]
    fn no_frontmatter_is_not_a_block() {
        assert!(block("no frontmatter here\n").is_none());
        // An unclosed block is not one either.
        assert!(block("---\nname: x\n").is_none());
    }

    #[test]
    fn a_scalar_block_is_not_a_mapping() {
        assert!(!is_mapping("just a string\n"));
        assert!(is_mapping("name: x\n"));
    }

    #[test]
    fn name_must_equal_the_unit_key() {
        let text = "---\nname: other\ndescription: d\n---\n";
        assert_eq!(
            validate_skill(text, "unit", "p/unit"),
            Err("p/unit: SKILL.md name must be 'unit'".to_owned())
        );
    }

    #[test]
    fn description_must_be_present_and_bounded() {
        let missing = "---\nname: unit\n---\n";
        assert_eq!(
            validate_skill(missing, "unit", "p/unit"),
            Err("p/unit: SKILL.md description must be a non-empty string".to_owned())
        );
        let long = format!("---\nname: unit\ndescription: {}\n---\n", "x".repeat(1025));
        assert_eq!(
            validate_skill(&long, "unit", "p/unit"),
            Err("p/unit: SKILL.md description exceeds 1024 characters".to_owned())
        );
    }

    #[test]
    fn empty_yaml_is_not_a_mapping() {
        assert!(validate_openai_yaml("", "p/x").is_err());
        assert!(validate_openai_yaml("name: x\n", "p/x").is_ok());
    }
}
