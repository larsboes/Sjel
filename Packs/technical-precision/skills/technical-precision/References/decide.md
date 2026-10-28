# Brainstorming, Evaluation, and Design

## Brainstorming

Separate divergence from convergence:

1. State the governing constraint when needed.
2. Generate 3-5 materially different options.
3. Make each option concrete enough to evaluate.
4. Remove duplicates disguised by different wording.
5. Converge using the user's real criteria when useful.

Do not criticize every option while generating it unless the user asks for immediate filtering.

## Evaluation

Define the decision before choosing a winner:

1. Identify mandatory constraints and eliminate options that fail them.
2. Use only criteria that can change the decision.
3. Rank or weight criteria when priorities differ.
4. Compare alternatives using the same assumptions and evidence.
5. Test sensitivity when uncertainty can reverse the ranking.
6. State the decisive trade-off.
7. Recommend one option when evidence supports it; otherwise state the decision boundary.

Possible criteria, only when relevant: correctness/requirement fit, operational complexity, observability, failure/recovery behavior, performance/resource cost, security/privacy/compliance, maintainability, migration cost, reversibility, and lock-in.

Include the status quo or "do nothing" only when genuinely viable.

## Architecture and design

Start with constraints and invariants, not technology names.

Check only the dimensions that matter:

- inputs, outputs, boundaries, trust zones;
- scale and latency/consistency needs;
- state ownership and durability;
- retries, idempotency, backpressure;
- failure isolation and partial-failure behavior;
- observability and operability;
- control plane vs. data plane when relevant;
- migration/reversibility.

Prefer the simplest design that satisfies the constraints. Do not add technology because it is fashionable or familiar.

## Recommendation

A concise decision usually contains:

- recommendation or leading option;
- decisive criteria/evidence;
- one or two material trade-offs;
- the condition that would change the choice when relevant.

Avoid large matrices unless they genuinely aid the decision.

## Stop rule

Stop when one option clearly dominates the decision-relevant criteria, or when remaining uncertainty cannot change the ranking.
