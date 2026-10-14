# cv pack

Build tailored CV PDFs from one master file, track applications, and compare the master with LinkedIn.

- **`cv-builder`**: runbook over the `cv` capability (`capabilities/cv/cv`). It builds a
  profile or a per-application CV, keeps the application tracker, compares the master with
  LinkedIn, and gives career guidance from that data. Application overrides reword facts from
  the master. They never add a fact.

Master CV content lives in the overlay (`$SJEL_PERSONAL_ROOT/data/cv/master_cv.yaml`, shape:
`capabilities/cv/master_cv.schema.yaml`) — nothing personal is baked into the skill.

## Activate

```bash
"$SJEL_ROOT/tools/packs.sh" link cv   # → ~/.claude/skills/cv-builder
"$SJEL_ROOT/tools/packs-codex" deploy cv  # → ~/.codex/skills/cv-builder
mkdir -p "$SJEL_PERSONAL_ROOT/data/cv"
cp "$SJEL_ROOT/capabilities/cv/master_cv.schema.yaml" \
   "$SJEL_PERSONAL_ROOT/data/cv/master_cv.yaml"        # then fill in your real content
```

## Attribution

Original work. License: MIT.
