# cognitive-load pack

Shape agent output for a reader who cannot afford ambiguity — whether that reader is a person
under load or a program with no human in the loop to resolve it.

Two skills, one matched pair, merged into one pack 2026-09-17:

- **`attention-control`**: the **shape** layer, for a reader with ADHD. Lead with the next
  action, number multi-step work, restate state every turn, state errors flat, cap lists at
  five, no preamble or closer. It is an output *style* rather than a task: it engages for the
  rest of the session and stays engaged until the reader lifts it. Invoke with
  `/attention-control`; it lifts on "stop attention control".
- **`asd-ste100`**: the **language** layer, for strings an agent must parse without ambiguity —
  tool descriptions, error messages, inter-agent instructions, status reports. ASD-STE100's
  structural rules: active voice, no phrasal verbs, one instruction per sentence, ≤20-word
  instruction and ≤25-word description caps, ≤3-word noun clusters, no semicolons, explicit
  ellipsis. Two modes, Strict and STE-flavored.

## Activate
```bash
"$SJEL_ROOT/tools/packs.sh" link cognitive-load    # → ~/.claude/skills/{attention-control,asd-ste100}
"$SJEL_ROOT/tools/packs-pi" deploy cognitive-load  # → registered in ~/.pi/agent/settings.json
```

`asd-ste100` is mandatory reading on the Claude Code surface: `~/.claude/CLAUDE.md` requires STE
discipline for tool descriptions, error strings and inter-agent instructions.

## Why these two are one pack, and one dependency

They used to be two packs, and `attention-control` restated STE's rules a second time — the same
numbers (20-word instruction cap, 3-word noun clusters, 6-sentence paragraphs) written down in
two repos, which is how a number drifts. `attention-control`'s own `references/doctrine.md`
always described **two layers** with two sources; what was wrong was that the language layer was
inlined instead of read.

So:

- Every word-, voice-, tense- and sentence-level rule lives in **`asd-ste100`** and nowhere else.
  `attention-control` states none of them and names where they are.
- The two are deployed together in every profile that holds either, which is what made one pack
  the honest boundary. It also removes a cross-pack dependency that could not have held in a
  public repo while `asd-ste100` was staged privately.

### The one rule where they appear to disagree

They can be active at once, and a reader will notice that `asd-ste100` says keep modality where
`attention-control` says delete hedging. Both hold, and the boundary is **whose uncertainty it
is** — stated in full in `attention-control`'s body so an agent holding both does not guess:

- **Uncertainty about a fact** follows `asd-ste100`. A hedge stays a hedge; "perhaps the build
  failed" never becomes "The build failed".
- **Uncertainty about the agent's own commitment** follows `attention-control`. "You might want
  to consider running the migration" becomes "Run the migration script. It takes about 2
  minutes".

## Attribution

Full grant text for both upstreams is in [`LICENSE`](LICENSE). Register entries:
`upstreams.toml [i-have-adhd]` and `[asd-ste100-skill]`.

- **`ayghri/i-have-adhd`** (MIT, Copyright (c) 2026 Ayoub Ghriss) is the source of the shape
  layer: action-first output, numbered steps, no preamble or closing offer. Adapted, not copied.
- **`danyuchn/asd-ste100-skill`** (MIT, Copyright (c) 2026 Dustin Yuchen Teng, base `7d4a135`)
  seeded `asd-ste100`, taken as inspiration and adapted into own work with no update path. The
  standard itself is **not** reproduced: ASD-STE100 is free to obtain but not free to
  redistribute, so `references/writing-rules.md` states the rule categories and cites the
  standard rather than copying its ~900-word approved dictionary.

`references/doctrine.md` records the reasoning behind the shape rules, and it is where an
unverifiable attribution used to sit: it credited the language layer to "`asd-ste100` by
L1nefeed", a name that matches no implementation on GitHub, while the register carried no entry
for either source. That claim is dropped — the layer it described is `asd-ste100`, whose real
attribution is above.
