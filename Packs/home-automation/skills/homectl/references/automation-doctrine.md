# Home Assistant automation reliability

Reusable rules for automation logic operated through homectl.

1. Define the safe state and require positive evidence before an unsafe action.
2. Reject unknown, unavailable, and stale sensor values before numeric conversion.
3. Use continuous guardians for conditions that must remain true over time.
4. Add hysteresis and debounce to thresholds.
5. Centralize each decision so several triggers cannot drift into different logic.
6. Notify when a protective action cannot reach its target.
7. Prefer local observed state over forecasts for stopping an action.
8. Keep operator-tunable thresholds in Home Assistant helpers.
9. Make commands idempotent and rate-limited.
10. Introduce new automations disabled or gated until failure paths are exercised.

The owning overlay records the concrete devices, entity IDs, thresholds, notification channels,
and live verification evidence. Public Sjel keeps only these reusable decision rules.
