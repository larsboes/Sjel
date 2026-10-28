# Debugging and Diagnosis

## Workflow

1. **Observed:** capture the exact symptom, error, failing assertion, metric, state, or unexpected output.
2. **Expected:** state what should happen under the relevant version/configuration.
3. **Scope:** identify the smallest component or boundary that can explain the mismatch.
4. **Hypotheses:** keep a short ranked set that explains the observations.
5. **Discriminating test:** choose the cheapest safe test that separates the leading hypotheses.
6. **Fix:** change the smallest thing that addresses the proven or well-supported cause.
7. **Verify:** rerun the failing path and check side effects/regressions.

## Diagnostic discipline

- Reproduce before changing the system when practical.
- Prefer interface observations: inputs, outputs, logs, metrics, traces, return codes, state transitions.
- Reduce to the smallest reproducible case when complexity hides the fault.
- Use known-good inputs/outputs to isolate components.
- Change one variable at a time when the goal is diagnosis.
- Distinguish symptom, trigger, contributing factor, and root cause.
- Do not claim root cause until evidence explains the observations and a fix/test verifies the causal path.
- If evidence supports only a likely cause, say so and name the confirming test.
- After repeated failed iterations, challenge the system model or assumed invariant instead of producing variants of the same fix.

## Output

Default to the exact failure, leading cause or ranked hypotheses, next discriminating test, then fix and verification only when supported.

Point to exact lines, values, state transitions, or error text when code/logs are available.

## Stop rule

Stop when one hypothesis explains the observations and verification supports the fix.
