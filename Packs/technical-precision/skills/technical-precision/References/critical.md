# Critical-Thinking Gate

Do not expose hidden chain-of-thought. Surface only decision-relevant assumptions, alternatives, evidence, tests, and uncertainty.

## Challenge the leading answer

1. Convert the task into a checkable claim, decision, or failure condition.
2. Identify scope variables that can change the answer: version, environment, configuration, timeframe, definitions, constraints, criteria.
3. Generate at least one plausible alternative that can explain the important evidence. Do not use a strawman.
4. Ask what observation would differ if the leading answer were wrong.
5. Prefer the cheapest safe discriminating test or source check.

For surprising, costly, irreversible, safety-relevant, or central claims, actively seek disconfirming evidence rather than only more support.

## Audit decisive evidence

Check:

- **Directness:** establishes the claim vs. merely suggests it.
- **Authority:** source controls or directly observes the fact.
- **Independence:** sources are independent rather than copies of one origin.
- **Scope:** exact version, environment, population, configuration, or timeframe matches.
- **Incentives:** source may emphasize one interpretation; treat this as context, not automatic disqualification.
- **Reproducibility:** decisive point can be reproduced by test, code, log, measurement, or independent evidence.

Do not turn source count into confidence when sources share one origin.

## Guard against common failures

Check only those that could change the answer:

- confirmation bias or anchoring;
- false dichotomy;
- correlation vs. causation;
- necessary vs. sufficient conditions;
- base-rate neglect;
- survivorship/selection bias;
- version leakage;
- absence-of-evidence errors;
- false precision.

## Stress-test recommendations

Ask:

1. Which criterion actually decides the choice?
2. What assumption is most load-bearing?
3. What failure mode makes the recommendation bad?
4. Is the decision reversible and what is the cost of being wrong?
5. Could realistic uncertainty change the ranking?

If small assumption changes reverse the result, report the recommendation as sensitive. When useful, give a boundary: "Choose A if X; choose B if Y."

## Calibrate the conclusion

- Directly documented/observed -> state as fact within scope.
- Several consistent observations -> well-supported inference.
- Plausible but untested -> hypothesis.
- Incomplete/conflicting evidence -> unresolved uncertainty.

State what evidence would change the recommendation only when decision-relevant.

## Stop rule

Do not turn critical thinking into analysis paralysis. Stop when plausible rivals are materially discriminated, remaining uncertainty cannot change the decision, or further evidence has low expected value relative to cost/risk.
