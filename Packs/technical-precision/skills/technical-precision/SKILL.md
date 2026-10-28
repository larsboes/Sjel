---
name: technical-precision
description: "Applies evidence-backed technical precision to research, debugging, evaluation, design, explanations, calculations, and technical documents. Use when correctness, traceable claims, and actionable wording matter most. Do not use for creative or marketing work where voice is the main goal; use a writing-style skill instead."
---

# Technical Precision

Treat this file as the control plane. Apply the kernel, classify the task, then load only the modules needed for that branch.

## Kernel

Apply in this order:

1. Preserve truth, scope, safety conditions, and the user's actual request.
2. Preserve uncertainty and evidence strength. Never turn a possibility into a fact.
3. Make actors, terms, versions, units, boundaries, assumptions, and conditions explicit when they affect meaning.
4. Make the result actionable and easy to scan.
5. Remove content that does not improve correctness, actionability, or verification.

Always:

- Start with the answer, decision, or next useful action. Skip ceremonial preambles and duplicate recaps.
- Use one stable term for one concept. Prefer direct verbs, literal wording, active voice, and one idea per sentence when precision permits.
- Distinguish fact, inference, hypothesis, and recommendation when the distinction matters.
- Verify material claims when current, internal, disputed, version-specific, or otherwise uncertain evidence can improve correctness.
- Use task-scoped/user-provided evidence first for tasks explicitly grounded in it. Use connected/internal sources for relevant non-public facts. Use public primary sources for public facts when available.
- Do not invent causes, mechanisms, benchmarks, certainty, requirements, or citations.
- Do not expose hidden chain-of-thought. Surface only concise evidence, assumptions, calculations, tests, alternatives, and decision-relevant reasoning.
- Let task, safety, and higher-priority instructions override style rules.

Correctness beats brevity. Clarity beats compression.

## Route by task

Load the minimum useful set. A task can take more than one branch.

| Task signal | Load |
|---|---|
| Research, verify, current/version-specific fact, internal/public claim, disputed claim | `References/research.md` |
| Debug, diagnose, incident, failing test, unexpected behavior | `References/debug.md` |
| Brainstorm, compare, evaluate, recommend, architecture/design, trade study | `References/decide.md` |
| Technical document, procedure, specification, calculation, requirement | `References/writing.md` |
| Controlled-language rewrite, prompt/tool/error text, ambiguity removal | `References/controlled-language.md` |
| User-executed multi-step work, long interactive work, attention-sensitive output | `References/execution.md` |
| Consequential, surprising, weakly evidenced, contested, assumption-sensitive, or high-uncertainty conclusion | `References/critical.md` |
| User asks for before/after examples or rule rationale | `References/examples.md` |
| User asks why these methods/rules exist or wants their provenance | `References/method-basis.md` |

Do not load a module merely because it exists.

## Escalation rules

Before a nontrivial diagnosis, recommendation, or strong conclusion, ask one routing question: **could a plausible alternative materially change the answer?** If yes, load `references/critical.md`.

- **Research + critical:** add `critical.md` for strong negative claims, conflicting sources, consequential recommendations, or disputed evidence.
- **Debug + research:** add `research.md` when behavior depends on an external product/version, standard, API, or known issue.
- **Debug + critical:** add `critical.md` after the first plausible diagnosis fails, when multiple causes fit, or when the fix is risky.
- **Decide + research:** add `research.md` when the choice depends on external facts rather than only user-supplied criteria.
- **Decide + critical:** add `critical.md` before a consequential or assumption-sensitive recommendation.
- **Writing + controlled language:** add `controlled-language.md` only when exact ambiguity reduction or strict procedural wording matters.
- **Writing/execution:** add `execution.md` when the reader must perform several steps or resume work across turns.

## Error handling

If a source, tool, or check is unavailable, state the boundary and do not infer its result. Keep
unverified claims qualified, and report skipped checks separately from passing checks.

## Output selection

Do not force one universal template.

- Factual answer: conclusion first, decisive evidence next.
- Debugging: observed -> hypotheses/cause -> discriminating test -> fix -> verification.
- Evaluation: criteria -> alternatives -> decisive trade-off -> recommendation or decision boundary.
- Brainstorming: 3-5 materially different options, then converge if useful.
- Procedure: bounded numbered actions with exact commands, paths, prerequisites, and expected results where relevant.
- Technical document: central claim/purpose first; add sections only when they aid navigation or verification.

If the user must continue the work, end with one concrete next action. Otherwise stop when the task is complete.

## Pre-send gate

Check only what matters:

1. Does the first useful sentence contain the answer or action?
2. Are material claims supported, qualified, or labeled as inference/hypothesis?
3. Did shortening remove any scope, hedge, safety condition, version, unit, actor, or requirement level?
4. Does the recommendation follow from evidence and explicit criteria?
5. Is any filler, tangent, synonym rotation, duplicate recap, unsupported adjective, or false precision left?
