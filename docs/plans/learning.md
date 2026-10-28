# Learning: teach, practice, and claims in the vault

Learning runs in the vault first. The `teach` and `practice` skills in `Packs/knowledge/` hold
the method, and the vault holds the result as claim notes. A Sjel learning surface is built
only when vault sessions show friction that a UI removes.

Decisions (2026-10-28):

- One claim note per proposition, `type: claim`, under `Knowledge/<domain>/Claims/`. The rule is
  in the vault contract's placement table.
- Vault-first, staged. A Sjel capability comes after real sessions, not before.
- The phone only reads notes for now. Phone practice decides between Obsidian, an Obsidian
  plugin and a Sjel client.

Prior art: `capabilities/learning` existed as a README with no code and was deleted on
2026-08-25 (`upstreams.toml`, `[slm-finetuning-talk]` row). The ideas used here come from
LearnHarness (`upstreams.toml`, `[learnharness]`).

## 1. Stage 1: the vault skills

Shipped, not yet committed: prediction step, thinking and confidence, and claim extraction in
`teach`. New `practice` skill. Claim row in the vault contract. `quiz` takes thinking and
confidence before the reveal. In Claude Code, `teach` uses `AskUserQuestion` with a confidence
question and the option note. The `🧩 Claims` view in `Knowledge.base` lists a concept's claims.

Open:

1. Run five real `teach` sessions, each followed by `practice`. Done when
   `Knowledge/Meta/Learning Log.md` has five teach lines and five practice lines.

Not checked: the `🧩 Claims` view renders only in Obsidian, and no one has looked at it yet. Also not checked: no session has run on the new skill text yet.

## 2. Stage 2: practice on the phone

Open. Decide after stage 1 between Obsidian mobile with a due-for-practice Base view, an
Obsidian plugin, and the Sjel agent on the phone ([`agent.md`](agent.md)). Done when one
practice session runs on the iPhone against vault claims.

## 3. Stage 3: a Sjel learning surface

Not started. Only if stages 1 and 2 show friction that a UI removes. It reads and writes the
same claim notes, so no data migrates.
