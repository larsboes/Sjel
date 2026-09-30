# Maintenance and provenance

Read this when re-vendoring the linter, auditing where a rule came from, or checking whether
this skill has drifted. None of it is needed to run the skill, which is why it is not in
SKILL.md.

## Local deltas against upstream

- **`checks.py::check_em_dash` fires on presence.** Upstream gated on `per_1k > threshold and
  count >= 2`, so a lone em dash was never reported. That contradicts the absolute-tell rule in
  SKILL.md. Verified against the calibration corpus: 0 of 24 human and 0 of 10 ESL-formal files
  pick up a false positive. **Re-apply this delta if the linter is ever re-vendored.**
- Additive entries in `ai_prose_patterns.json`.
- **Inline HTML ignore directives removed (2026-09-26).** The upstream linter supports
  `human-voice` comments, but Sjel does not. Keep scanner policy outside prose files.
- **One user-facing string corrected in `patterns.py` (2026-09-17).** Its pattern-file-not-found
  message told the user to fall back to `references/ai-tells.md`, a path that has never existed in
  this repository. It now names `references/tells.md`, which is the catalog it means. A string, not
  behaviour — but it is a string a user reads at the moment the linter has already failed, which is
  the worst moment to send them to a file that is not there. **Re-apply if re-vendored.**

## What the calibration corpus proves, and does not

The corpus is hand-authored by the linter's own upstream to exhibit the tells it looks for. Its
numbers measure internal consistency, never real-world accuracy. The published ablation reports
AUC 1.0 with zero drop for every category, which means the corpus is saturated, not that the
linter is perfect. Do not cite it as a benchmark.

## Provenance

Merged 2026-07-25 from three MIT upstreams, superseding the former `unslop-text` skill, which
moved here from the `unslop` pack with its history preserved via `git mv`:

- [stephenoffer/human-voice](https://github.com/stephenoffer/human-voice) pinned `9bcba2f` —
  the linter (`scripts/human_voice_linter/`, `detect_ai_prose.py`, `ai_prose_patterns.json`),
  register profiles, over-correction catalog, anti-hallucination protocol. Local deltas are
  listed above, plus additive pattern entries.
- [ryanthedev/oberskills](https://github.com/ryanthedev/oberskills) pinned `5050537`, the
  `write` skill — the mode router, reader-job taxonomy, voice profiles, review protocol. MIT is
  declared in `.claude-plugin/plugin.json` only, with no LICENSE file at the repo root, so its
  contributions are paraphrased rather than copied verbatim.
- [JCarterJohnson/vibecoded-design-tells](https://github.com/JCarterJohnson/vibecoded-design-tells)
  pinned `f7c4aef` — the cited-vs-matched weighting, the density model, the tell catalog with
  real quotes, and the over-corrected-register framing.

[hardikpandya/stop-slop](https://github.com/hardikpandya/stop-slop) was evaluated and declined a
second time. Its distinctive contributions (false agency, narrator-from-a-distance, negative
listing, dramatic fragmentation, vague declaratives) are already present as pattern categories
here, and its absolutist framing ("kill all adverbs", "no em dashes ever", "two items beat
three") manufactures exactly the uniform signature this skill exists to prevent.

## Re-verify on drift

```bash
python3 scripts/detect_ai_prose.py --help    # prints without error
```

Metadata and body-budget validation is **not** run from here. It used to be, as a command
reaching into `skill-creator`'s own `scripts/validate_metadata.py` by repo path — a dependency on
a sibling skill's file, which is exactly what a skill must not have: it resolves in the Sjel
checkout and nowhere else, so a skill copied into a harness carried a verification step that
could not run. `tools/check-skill-metadata.sh` now runs that same validator over every authored
SKILL.md on every push, which is strictly stronger than checking one skill when someone
remembers to. The validator has one owner, and this skill is not it.
