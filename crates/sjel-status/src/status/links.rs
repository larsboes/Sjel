use super::*;

/// Operator-pinned links the dashboard shows on home: `<overlay>/config/links.toml`,
/// read fresh on every request so editing the file is the whole workflow.
///
/// The parser accepts the same constrained TOML the repo already standardizes for
/// shell-side parsing (tools/lib/toml.sh): `[[links]]` tables of single-line quoted
/// scalars. Hand-rolled rather than a toml crate for the same reason backup.rs parses
/// its timestamps by hand — the format is ours and fixed, and the alternative costs a
/// dependency for three keys.
///
/// A missing file (or an unset overlay) is a deployment that pins nothing — `links: []`,
/// never an error: this card is decoration, and the overlay's absence already gets
/// reported by the surfaces that own that concern.
#[derive(serde::Serialize, PartialEq, Debug, Default, Clone)]
pub(crate) struct PinnedLink {
    pub(crate) name: String,
    pub(crate) url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) note: String,
}

pub(crate) fn parse_links(text: &str) -> Vec<PinnedLink> {
    let mut out: Vec<PinnedLink> = Vec::new();
    let mut current: Option<PinnedLink> = None;
    let push = |entry: Option<PinnedLink>, out: &mut Vec<PinnedLink>| {
        if let Some(link) = entry {
            // Only complete entries with a web URL render; anything else is a config
            // typo the card must not amplify into a dead or javascript: href.
            if !link.name.is_empty()
                && (link.url.starts_with("https://") || link.url.starts_with("http://"))
            {
                out.push(link);
            }
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[links]]" {
            push(current.take(), &mut out);
            current = Some(PinnedLink::default());
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim() {
            "name" => entry.name = value,
            "url" => entry.url = value,
            "note" => entry.note = value,
            _ => {}
        }
    }
    push(current.take(), &mut out);
    out
}

pub(crate) async fn links_handler() -> axum::Json<serde_json::Value> {
    let links = overlay_root()
        .ok()
        .map(|root| root.join("config/links.toml"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse_links(&text))
        .unwrap_or_default();
    axum::Json(serde_json::json!({ "links": links }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_entries_and_keeps_order() {
        let text = r#"
# operator links
[[links]]
name = "Thesis exposition"
url  = "https://node.example.ts.net:9443/"
note = "tailnet only"

[[links]]
name = "Vaultwarden"
url = "https://node.example.ts.net/"
"#;
        let links = parse_links(text);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].name, "Thesis exposition");
        assert_eq!(links[0].note, "tailnet only");
        assert_eq!(links[1].note, "");
    }

    #[test]
    fn drops_incomplete_and_non_web_entries() {
        let text = r#"
[[links]]
name = "no url at all"

[[links]]
name = "wrong scheme"
url = "javascript:alert(1)"

[[links]]
url = "https://nameless.example/"

[[links]]
name = "survivor"
url = "http://ok.example/"
"#;
        let links = parse_links(text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].name, "survivor");
    }

    #[test]
    fn stray_keys_and_garbage_lines_are_ignored() {
        let text = "name = \"before any table\"\n[[links]]\nname = \"a\"\nurl = \"https://a.example/\"\ncolor = \"red\"\nnot a kv line\n";
        let links = parse_links(text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].name, "a");
    }

    #[test]
    fn empty_input_yields_no_links() {
        assert!(parse_links("").is_empty());
        assert!(parse_links("# only comments\n").is_empty());
    }
}
