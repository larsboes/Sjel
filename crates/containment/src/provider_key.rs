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

/// One `--credential` argument, split into the catalogue entry to read and the variable to set.
#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    pub credential_id: String,
    pub variable: String,
}

/// Read one `--credential` argument. `deepseek` is the short form: the catalogue entry
/// `inference-deepseek` into `DEEPSEEK_API_KEY`. `<id>=<VARIABLE>` reads that entry, for a harness
/// whose own variable is not derived from an inference slug, for example
/// `copilot-github=COPILOT_GITHUB_TOKEN`.
pub fn target(spec: &str) -> Result<Target, String> {
    match spec.split_once('=') {
        Some((id, variable)) => {
            if !valid_id(id) {
                return Err(format!(
                    "invalid catalogue entry {id:?}; run `sjel provider list`"
                ));
            }
            if !variable.starts_with(|c: char| c.is_ascii_uppercase())
                || !variable
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(format!(
                    "invalid variable {variable:?}; an environment name is upper case A-Z, 0-9 and _"
                ));
            }
            Ok(Target {
                credential_id: id.to_owned(),
                variable: variable.to_owned(),
            })
        }
        None => {
            let variable = env_name(spec).ok_or_else(|| {
                format!("invalid provider slug {spec:?}; run `sjel provider list`")
            })?;
            Ok(Target {
                credential_id: format!("inference-{spec}"),
                variable,
            })
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// The key of one provider, read from the same catalogue `sjel provider` reads. The caller keeps
/// the value in its own process and passes it to the child's environment, so it appears in no
/// command line, shell history or parent environment.
pub fn resolve(spec: &str) -> Result<(String, String), String> {
    let target = target(spec)?;
    let value = sjel_inference::credential_value(&target.credential_id).ok_or_else(|| {
        format!(
            "no {} credential; run `sjel provider login {}`",
            target.credential_id,
            target.credential_id.trim_start_matches("inference-")
        )
    })?;
    Ok((target.variable, value))
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

    #[test]
    fn the_short_form_names_the_inference_entry_and_its_variable() {
        assert_eq!(
            target("deepseek"),
            Ok(Target {
                credential_id: "inference-deepseek".into(),
                variable: "DEEPSEEK_API_KEY".into(),
            })
        );
    }

    #[test]
    fn a_named_pair_reads_that_entry_into_that_variable() {
        assert_eq!(
            target("copilot-github=COPILOT_GITHUB_TOKEN"),
            Ok(Target {
                credential_id: "copilot-github".into(),
                variable: "COPILOT_GITHUB_TOKEN".into(),
            })
        );
        assert!(target("copilot-github=gh_token").is_err());
        assert!(target("copilot-github=").is_err());
        assert!(target("../credentials=GH_TOKEN").is_err());
        assert!(target("=GH_TOKEN").is_err());
    }
}
