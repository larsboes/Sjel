//! Documentation slug resolution and GitHub source URLs.

pub(crate) fn doc_slug(href: &str, file: &str) -> Option<String> {
    if href.starts_with('#') || href.starts_with("//") || href.contains("://") {
        return None;
    }
    let path = href.split(['?', '#']).next()?;
    if path.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if !path.starts_with('/') {
        if let Some((parent, _)) = file.rsplit_once('/') {
            parts.extend(parent.split('/'));
        }
    }
    for part in path.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    let joined = parts.join("/");
    let path = joined.strip_prefix("docs/")?;
    let mut slug = path.strip_suffix(".md")?.to_owned();
    if slug == "README" {
        slug = "index".into();
    } else if let Some(parent) = slug.strip_suffix("/index") {
        slug = parent.to_owned();
    }
    Some(slug)
}

pub(crate) fn source_url(href: &str, file: &str) -> Option<String> {
    if href.starts_with("https://") || href.starts_with("http://") || href.starts_with("mailto:") {
        return Some(href.to_owned());
    }
    if href.starts_with('#') || href.starts_with("//") {
        return None;
    }
    let path = href.split(['?', '#']).next()?;
    let mut parts = Vec::new();
    if !path.starts_with('/') {
        if let Some((parent, _)) = file.rsplit_once('/') {
            parts.extend(parent.split('/'));
        }
    }
    for part in path.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    let suffix = href.find(['?', '#']).map_or("", |at| &href[at..]);
    Some(format!(
        "https://github.com/larsboes/Sjel/blob/main/{}{suffix}",
        parts.join("/")
    ))
}
