//! The SKILL.md frontmatter contract, read here and nowhere else.
//!
//! This module is a re-export: the parser moved to the `sjel-skill-metadata` crate on
//! 2026-10-08, because `libs/agent`'s skills extension reads the same block. One parser for
//! two readers — a skill that the Pack engine deploys while the agent sees no frontmatter is
//! a skill that silently never reaches a system prompt.
//!
//! The call sites keep saying `frontmatter::…`, which is the shape they were written in.

pub use sjel_skill_metadata::{validate_openai_yaml, validate_skill};
