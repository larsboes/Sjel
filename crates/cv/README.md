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

Use `sjel cv <args>`; `sjel help cv` lists the verbs without reading personal data.
The examples below use the public entry point.

```bash
sjel cv build --all                                  # every profile; one page unless page_limit says otherwise
sjel cv build --profile swe --lang de
sjel cv app new acme --company Acme --role "Platform Engineer" --profile swe
sjel cv app build acme
sjel cv edit get | $EDITOR … | sjel cv edit put           # or the /cv page: form or YAML
sjel cv linkedin sheet --lang en
```

Requires `typst` and `cargo`; `curl` for LinkedIn. Shape of the data:
`master_cv.schema.yaml`. Manual: [`docs/capabilities/cv.md`](../../docs/capabilities/cv.md).
