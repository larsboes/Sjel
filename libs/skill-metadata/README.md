# libs/skill-metadata

The `SKILL.md` frontmatter contract, read in one place.

A skill file opens with a `---` block holding at least `name` and `description`, and everything
after it is the document. Two readers need those two keys and must agree on them:

- `tools/sjel-cli` decides whether a Pack's skill deploys.
- `libs/agent`'s `skills` extension decides what goes into a system prompt.

A skill that the engine deploys while the extension sees no frontmatter is a skill that silently
never reaches a model, which is why this is a crate rather than a function in each of them.

## What it parses

Not YAML, and it says so rather than pretending: a top-level `key:`, a folded or literal block
scalar, and a quoted or plain inline scalar. A nested mapping is not descended into, so a `name:`
under another key does not count, and an inline ` #` comment is cut the way YAML cuts it. That is
exactly what `Bun.YAML.parse` plus the two-key read did before this, and what
`Packs/writing/skills/skill-creator/scripts/validate_metadata.py` checks the same way.

```rust
use sjel_skill_metadata::{block, body, scalar, validate_skill};

let text = std::fs::read_to_string("Packs/.../SKILL.md")?;
validate_skill(&text, "my-skill", "Packs/.../SKILL.md")?;   // the engine's rules, verbatim
let description = scalar(block(&text).unwrap(), "description").unwrap();
let document = body(&text);
```

`body` is the text after the closing delimiter, or the whole file when there is none; `block` and
`split` return the frontmatter. No dependencies.
