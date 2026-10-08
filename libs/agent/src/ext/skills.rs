//! Skills from `Packs/` (F3).
//!
//! Each enabled skill contributes its `name` and its `description` to the system prompt, and
//! nothing else of its `SKILL.md`. The document itself arrives only when the model asks for it by
//! name, which is the whole point: a session with twenty skills enabled pays for twenty
//! descriptions, not twenty documents.
//!
//! The frontmatter is read with the same parser the Pack engine deploys with
//! (`sjel-skill-metadata`), because the two must agree — a skill that deploys while this sees no
//! frontmatter is a skill that silently never reaches a prompt.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sjel_agent::{Extension, Tool};

/// The name `agent.toml` and `--ext` use. Not on by default: a run names the skills it wants.
pub const NAME: &str = "skills";

/// One enabled skill: what the prompt says about it, and where its document lives.
struct Skill {
    name: String,
    description: String,
    document: PathBuf,
}

/// The extension, holding what [`Skills::load`] read.
pub struct Skills {
    skills: Vec<Skill>,
}

impl Skills {
    /// Reads each path: a directory holding `SKILL.md`, or that file itself.
    ///
    /// A path that will not read, or whose frontmatter fails the validation the Pack engine
    /// applies, comes back as a problem rather than an error. One broken skill among twenty is
    /// not a reason to refuse the run, and a skill that vanishes without a word is worse than a
    /// run that does not start — so the caller reports every problem (AGT-13).
    pub fn load(paths: &[String]) -> (Self, Vec<String>) {
        let mut skills = Vec::new();
        let mut problems = Vec::new();
        for given in paths {
            match read_skill(given) {
                Ok(skill) => skills.push(skill),
                Err(problem) => problems.push(problem),
            }
        }
        (Self { skills }, problems)
    }
}

fn read_skill(given: &str) -> Result<Skill, String> {
    // A path in a config file is usually written `~/…`, and an unexpanded tilde would be read
    // as a relative path and reported as missing.
    let path = sjel_config::expand_tilde(given);
    let (document, directory) = if path.is_dir() {
        (path.join("SKILL.md"), path.clone())
    } else {
        let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        (path.clone(), parent)
    };
    let text =
        std::fs::read_to_string(&document).map_err(|e| format!("{}: {e}", document.display()))?;
    // The Pack engine's rule, reused: `name` names the directory, and a description is present
    // and bounded. Anything that would not deploy is not offered to the model either.
    let key = directory
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    sjel_skill_metadata::validate_skill(&text, &key, &document.display().to_string())?;
    let description = sjel_skill_metadata::block(&text)
        .and_then(|block| sjel_skill_metadata::scalar(block, "description"))
        .unwrap_or_default();
    Ok(Skill {
        name: key,
        description,
        document,
    })
}

impl Extension for Skills {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        // The tool owns the name-to-document map rather than borrowing the extension: both live
        // in the same agent, and one path per skill is cheaper than a self-reference.
        vec![Box::new(SkillTool {
            documents: self
                .skills
                .iter()
                .map(|skill| (skill.name.clone(), skill.document.clone()))
                .collect(),
        })]
    }

    fn system(&self, prompt: &mut String) {
        if self.skills.is_empty() {
            return;
        }
        prompt.push_str(
            "\n\n# Skills\n\nA skill is a method this repository already settled on. When one of \
             them covers the task, call the `skill` tool with its name before you start, and \
             follow what it says.\n\n",
        );
        for skill in &self.skills {
            prompt.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
    }
}

/// Hands back one skill's document, by the name the prompt used.
struct SkillTool {
    documents: Vec<(String, PathBuf)>,
}

impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        "skill"
    }
    fn description(&self) -> &'static str {
        "Return a skill's document by name. The system prompt lists the skills this run has, with \
         one line each; this returns the whole method, which is long enough that it is not in the \
         prompt."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["name"], "properties": {
            "name": { "type": "string", "description": "A skill name from the system prompt's Skills list." }
        }})
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or("`name` is required")?;
        let Some((_, document)) = self
            .documents
            .iter()
            .find(|(known, _)| known.as_str() == name)
        else {
            let known: Vec<&str> = self
                .documents
                .iter()
                .map(|(known, _)| known.as_str())
                .collect();
            return Err(format!(
                "there is no skill named `{name}`; this run has: {}",
                known.join(", ")
            ));
        };
        let text = std::fs::read_to_string(document)
            .map_err(|e| format!("{}: {e}", document.display()))?;
        Ok(sjel_skill_metadata::body(&text)
            .trim_start_matches('\n')
            .to_owned())
    }
    fn read_only(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY_MARKER: &str = "THE-BODY-OF-THE-DOCUMENT";

    /// A skill directory under its own temp parent, so tests do not collide.
    fn skill(parent: &str, name: &str, frontmatter: &str) -> String {
        let dir = std::env::temp_dir()
            .join(format!("sjel-agent-skills-{}-{parent}", std::process::id()))
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\n{frontmatter}---\n\n{BODY_MARKER}\n\n1. Step one.\n"),
        )
        .unwrap();
        dir.to_string_lossy().into_owned()
    }

    fn good(parent: &str, name: &str, description: &str) -> String {
        skill(
            parent,
            name,
            &format!("name: {name}\ndescription: {description}\n"),
        )
    }

    fn tool(skills: &Skills) -> Box<dyn Tool> {
        skills
            .tools()
            .into_iter()
            .find(|tool| tool.name() == "skill")
            .expect("the skills extension carries one tool")
    }

    #[test]
    fn the_prompt_carries_each_skill_and_none_of_their_bodies() {
        let alpha = good("prompt", "alpha", "does the first thing");
        let beta = good("prompt", "beta", "does the second");
        let (skills, problems) = Skills::load(&[alpha, beta]);
        assert!(problems.is_empty(), "{problems:?}");

        let mut prompt = String::from("be terse");
        skills.system(&mut prompt);
        for name in ["alpha", "beta"] {
            assert!(prompt.contains(name), "{prompt}");
        }
        assert!(prompt.contains("does the first thing"), "{prompt}");
        assert!(prompt.contains("does the second"), "{prompt}");
        // AGT-12: name and description, and not one byte of the document.
        assert!(
            !prompt.contains(BODY_MARKER),
            "a skill body reached the prompt: {prompt}"
        );
        // The tool is the other half: both are reachable by the name the prompt used.
        let tool = tool(&skills);
        for name in ["alpha", "beta"] {
            assert!(
                tool.run(&json!({ "name": name })).is_ok(),
                "{name} is not reachable"
            );
        }
    }

    #[test]
    fn a_broken_skill_is_named_and_the_run_goes_on() {
        let fine = good("broken", "fine", "works");
        let bad = skill("broken", "bad", "name: bad\ndescription:\n");
        let (skills, problems) = Skills::load(&[fine, bad.clone()]);
        let mut prompt = String::new();
        skills.system(&mut prompt);
        assert!(
            prompt.contains("fine"),
            "the good skill was dropped too: {prompt}"
        );
        assert!(
            !prompt.contains("bad"),
            "the broken skill reached the prompt: {prompt}"
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("bad"), "{problems:?}");
        assert!(
            problems[0].contains("description must be a non-empty string"),
            "{problems:?}"
        );

        // A directory that is not there, and one with no frontmatter at all.
        let (_, problems) =
            Skills::load(&["/nonexistent/skill".to_owned(), BODY_MARKER.to_owned()]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("/nonexistent/skill"), "{problems:?}");
    }

    #[test]
    fn the_tool_returns_the_document_without_its_frontmatter() {
        let (skills, _) = Skills::load(&[good("tool", "gamma", "the third thing")]);
        let got = tool(&skills)
            .run(&json!({ "name": "gamma" }))
            .expect("the skill is there");
        assert!(got.contains(BODY_MARKER), "{got}");
        assert!(!got.contains("description: the third thing"), "{got}");
        assert!(!got.starts_with("---"), "{got}");
    }

    #[test]
    fn an_unknown_name_lists_what_there_is() {
        let (skills, _) = Skills::load(&[good("unknown", "delta", "the fourth thing")]);
        let err = tool(&skills).run(&json!({ "name": "nope" })).unwrap_err();
        assert!(err.contains("no skill named `nope`"), "{err}");
        assert!(err.contains("delta"), "{err}");
    }

    #[test]
    fn no_skills_leaves_the_prompt_alone() {
        let (skills, problems) = Skills::load(&[]);
        assert!(problems.is_empty());
        let mut prompt = String::from("be terse");
        skills.system(&mut prompt);
        assert_eq!(
            prompt, "be terse",
            "an empty skill set still edited the prompt"
        );
    }
}
