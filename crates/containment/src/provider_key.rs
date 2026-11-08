//! The provider key a contained harness needs, resolved outside the sandbox and handed to the
//! child as one environment variable.

/// The variable a harness reads its provider key from: `deepseek` is `DEEPSEEK_API_KEY` and
/// `opencode-go` is `OPENCODE_GO_API_KEY`. A slug the credential catalogue cannot hold is None.
pub fn env_name(slug: &str) -> Option<String> {
    let valid = !slug.is_empty()
        && slug.len() <= 64
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if !valid {
        return None;
    }
    let mut name = String::with_capacity(slug.len() + "_API_KEY".len());
    for character in slug.chars() {
        name.push(if character == '-' {
            '_'
        } else {
            character.to_ascii_uppercase()
        });
    }
    name.push_str("_API_KEY");
    Some(name)
}

/// The key of one provider, read from the same catalogue `sjel provider` reads. The caller keeps
/// the value in its own process and passes it to the child's environment, so it appears in no
/// command line, shell history or parent environment.
pub fn resolve(slug: &str) -> Result<(String, String), String> {
    let name = env_name(slug)
        .ok_or_else(|| format!("invalid provider slug {slug:?}; run `sjel provider list`"))?;
    let id = format!("inference-{slug}");
    let value = sjel_inference::credential_value(&id)
        .ok_or_else(|| format!("no {id} credential; run `sjel provider login {slug}`"))?;
    Ok((name, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_names_the_variable_the_harness_reads() {
        assert_eq!(env_name("deepseek").as_deref(), Some("DEEPSEEK_API_KEY"));
        assert_eq!(
            env_name("opencode-go").as_deref(),
            Some("OPENCODE_GO_API_KEY")
        );
    }

    #[test]
    fn a_slug_outside_the_catalogue_is_refused() {
        assert!(env_name("").is_none());
        assert!(env_name("deepseek;rm").is_none());
        assert!(env_name("../../credentials").is_none());
        assert!(resolve("deepseek;rm").is_err());
    }
}
