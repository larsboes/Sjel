# cv

One master CV, tagged by profile and priority, built into one-page variants per profile or per
application, plus the full CV that LinkedIn mirrors.

**Why build it:** the job is a filter over one personal file, and Typst does the typesetting
(`upstreams.toml [typst]`). Two earlier attempts did not last: a LaTeX system with a macro
branch per target, and a React + Gemini rebuild far larger than the requirement. The data
stays in the overlay (`$SJEL_PERSONAL_ROOT/data/cv/`); this directory holds code and the schema.

[`RUNBOOK.md`](RUNBOOK.md) is the workflow over these verbs — building for a posting, curating the
master, tracking applications, the LinkedIn import and diff, and the rules that keep a generated
CV honest. It was an agent skill until 2026-10-24 and moved here rather than leaving with the Pack.

```bash
cv build --all                                  # every profile; one page unless page_limit says otherwise
cv build --profile swe --lang de
cv app new acme --company Acme --role "Platform Engineer" --profile swe
cv app build acme
cv edit get | $EDITOR … | cv edit put           # or the /cv page: form or YAML
cv linkedin sheet --lang en
```

Requires `typst` and `cargo`; `curl` for LinkedIn. Shape of the data:
`master_cv.schema.yaml`. Manual: [`docs/capabilities/cv.md`](../../docs/capabilities/cv.md).
Open work: [`ISA.md`](ISA.md).
