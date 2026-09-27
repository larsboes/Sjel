# cv pack

Build job-tailored CV PDFs from one profile-tagged master file, via Typst.

- **`cv-builder`** — runbook over the `cv` capability (`capabilities/cv/cv`): pick a profile
  tag, pick a language, build, hand back the PDF path. No content-generation or AI-tailoring —
  every build is a deterministic filter over the master file.

Master CV content lives in the overlay (`$SJEL_PERSONAL_ROOT/data/cv/master_cv.yaml`, shape:
`capabilities/cv/master_cv.schema.yaml`) — nothing personal is baked into the skill.

## Activate

```bash
"$SJEL_ROOT/tools/packs.sh" link cv   # → ~/.claude/skills/cv-builder
"$SJEL_ROOT/tools/packs-codex" deploy cv  # → ~/.agents/skills/cv-builder
mkdir -p "$SJEL_PERSONAL_ROOT/data/cv"
cp "$SJEL_ROOT/capabilities/cv/master_cv.schema.yaml" \
   "$SJEL_PERSONAL_ROOT/data/cv/master_cv.yaml"        # then fill in your real content
```

## Attribution

Original work. License: MIT.
