# Skill evaluation

Evals are how a skill earns its context cost — and how a de-prescription pass proves a cut
was safe. Format and loop per agentskills.io's evaluating-skills guide (snapshot 2026-08-09);
baseline-first RED discipline adapted idea-only from obra/superpowers' skill-testing method.

**Run them with `tools/skill-eval`**, the executor for everything below:

```bash
tools/skill-eval list                   # every suite, its cases, whether a baseline is recorded
tools/skill-eval check --all            # validate the suites (file-based; this half runs in CI)
tools/skill-eval run <skill> --case 2    # four model calls when graded
tools/skill-eval run <skill> --case 2 --repeat 3  # opt-in: twelve model calls
```

`check` is in `repo-gates`; `run` is opt-in and manual, because a gate that needs a model and
costs money is a gate that flakes. The tool's own header states what the delta does and does not
measure — including why the two arms cannot be isolated from the machine's other installed skills.

## Contents
- Baseline first: the RED phase
- The evals.json format
- The A/B loop and workspace
- Assertions — and the near-miss rule for triggers
- Grading and the benchmark delta
- Analyzing patterns
- Three test tiers
- Under/over-triggering: signal → fix

## Baseline first: the RED phase

Before writing (or fattening) a skill, run the task **without it** in a fresh-context
subagent and capture the failure verbatim — including the model's rationalizations for
skipping steps. Then write only enough instruction to close that observed failure, and re-run
to confirm. A skill written against imagined failures documents needs that never existed; a
discipline skill written against harvested rationalizations can rebut them specifically.
Fresh context matters: leftover authoring context masks gaps in the written instructions.
If the baseline already meets the assertions, record that result instead of inventing a failure.
A runnable tool can add capability without establishing a reasoning-quality gain.

## The evals.json format

Hand-authored test cases live at `evals/evals.json` inside the skill directory. **One schema** —
the two suites in this repo used different key names (`skill`+`cases` vs `skill_name`+`evals`)
until 2026-09-17, and nothing read either, so the divergence survived unnoticed.
`tools/skill-eval check` now fails the old keys by name.

```json
{
  "skill_name": "example-skill",
  "baseline": { "captured": "YYYY-MM-DD", "method": "how the unaided run was taken", "failure": "what it got wrong" },
  "results":  { "recorded": "YYYY-MM-DD", "iteration": "<skill>-workspace/iteration-N/" },
  "evals": [
    {
      "id": 1,
      "prompt": "realistic user message, the kind someone actually types",
      "expected_output": "human-readable description of success",
      "files": ["evals/files/fixture.csv"],
      "assertions": [
        "programmatically or observably verifiable statement",
        "another specific, countable check"
      ]
    }
  ],
  "trigger_tests": {
    "should_trigger": ["the words a user would actually say"],
    "should_not_trigger": ["a boundary case (near-miss: the-sibling-skill)"]
  }
}
```

Required: `skill_name` (must equal the directory), a non-empty `evals` array, and per case a
unique `id`, a non-empty `prompt`, and at least one assertion. Everything else is optional and is
reported when absent: a suite with no `baseline` warns, because a suite not written against a real
failure cannot be trusted — and its author's own suite would fail a rule enforced from day one.
`files` resolve against the skill directory and must exist.

Start with 2–3 cases: vary phrasing and formality, include one edge case, use realistic
context (paths, names). Write `assertions` after the first run — "good" is defined by what
the runs actually produce.

## The A/B loop and workspace

Run every case twice from clean context — with the skill, and without it (or against a
snapshot of the previous version when improving). Results live in a workspace **next to** the
skill directory, one `iteration-N/` per pass:

```
<skill>-workspace/iteration-1/
├── eval-<case-slug>/
│   ├── with_skill/{outputs/, timing.json, grading.json}
│   └── without_skill/{outputs/, timing.json, grading.json}
└── benchmark.json
```

The runner launches native Claude processes. Each arm gets a fresh directory, but installed
global context remains shared. It does not provide an isolated owned-revision comparison.
For `--repeat N` with N greater than one, insert `sample-<N>/` below each case directory.
The default one-sample paths remain unchanged.

Each arm records task `timing.json`, `grading-timing.json`, raw `grading-result.txt`, validated
`grading.json` and `sample.json` with explicit case, sample and arm identity. Task and grading
usage and costs stay separate; combined totals include both. Unknown billing metadata is an
error, not a free call. `--no-grade` records an intentional skip with no pass-rate observation.
A graded case costs four model calls per repeat, not a fixed currency amount.

## Assertions — and the near-miss rule for triggers

Good assertions are verifiable and specific ("output file is valid JSON", "report includes at
least 3 recommendations"); weak ones are vague ("output is good") or brittle (exact-phrase
matches). Not everything needs one — style and feel belong to human review.

For **trigger tests** (does the description fire correctly), the should-NOT-trigger prompts
must be *near-misses*: boundary-ambiguous requests a sibling skill should win ("migrate my
React app to Vite" against an Angular-migration skill). Obviously irrelevant prompts test
nothing — any description passes them.

## Grading and the benchmark delta

Grade each assertion PASS/FAIL **with quoted evidence** from the output; a section labeled
"Summary" containing one vague sentence fails an "includes a summary" assertion — the label
is not the substance. Use a script for mechanical checks (valid JSON, file exists, line
counts); an LLM for observational ones; **blind comparison** (two outputs, judge not told
which version produced which) for holistic quality between versions.

The grader must cover every requested assertion exactly once, with PASS or FAIL and nonempty
evidence. Malformed, missing, duplicate or unknown assertions make the entire response UNGRADED.
Incomplete grading retains its output and billed cost, but the graded run exits unsuccessfully.
Inspect evidence: structural validation does not prove a model's verdict is correct. The native
grader sees returned text, not an independent verification of generated files.

`benchmark.json` records the actual UTC date, sample identities, task/grading cost totals and
per-case/per-arm descriptive statistics: n, mean, sample standard deviation, min and max.
Missing or ungraded pass rates are not successes. Empty data is null; one observation has
unknown standard deviation, not zero uncertainty. Do not pool distinct cases into an implied
repeated experiment. Raw aggregate assertion counts are supplementary, not statistical proof.

Use deltas as evidence to inspect, not an automatic verdict. Repetition does not establish
significance, causality or independence. Trigger selection, holistic blind comparison and
controlled before/candidate snapshots require their own procedures and evidence.

## Analyzing patterns

- Assertions that pass in **both** arms: do not claim an instruction gain. Retain useful contract
  regressions without treating them as evidence that more guidance was needed.
- Assertions that fail in **both** arms: broken assertion, too-hard case, or wrong check — fix
  before the next iteration.
- Pass-with / fail-without: the skill's actual value; understand which instruction did it.
- Inconsistent across runs: inspect transcripts. The model, task or grader may explain the
  variation; a larger standard deviation alone does not identify the cause.
- Token/time outliers: read that run's transcript for the bottleneck.
- Feed failed assertions + human feedback + transcripts to an LLM with the current SKILL.md
  and ask for changes that **generalize** (no per-test patches), keep the skill lean (remove
  instructions transcripts show as wasted work), and explain why over bare directives.

Stop iterating when human feedback comes back empty or deltas stop moving.

## Three test tiers

| Tier | Goal | Example |
|------|------|---------|
| 1 Triggering | Fires when it should, silent when it shouldn't | Paraphrases should-trigger; near-misses should NOT |
| 2 Functional | Correct output | evals.json assertions pass |
| 3 Comparison | Beats no-skill baseline | benchmark delta positive on pass rate at acceptable token/time cost |

## Under/over-triggering: signal → fix

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Never loads when it should | Description too generic, missing the user's actual words | Add concrete trigger phrases + file types/terms |
| Loads for unrelated queries | Description too broad | Add a negative trigger naming the sibling to use instead |
| User manually invokes every time | Same as under-triggering | Same fix; re-run tier-1 |
| "Stops working" mid-session | Content still in context; model choosing otherwise — or description budget-dropped in a large install | Sharpen description/instructions; check `/doctor` for budget drops; re-invoke after compaction |

Debug fast: ask a fresh instance "when would you use the `<name>` skill?" — it quotes its
understanding of the description back, showing exactly what's missing.
