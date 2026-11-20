# Scenario template

Copy this into `scenarios/<asset>.md` and fill every section. Write "unknown" where the evidence
is missing. Do not leave a section out.

```markdown
# Compromise: <asset>

Date: <YYYY-MM-DD>. Sources: <commands run, files read>.

## 1. Asset and trust
What the asset is. The data classes it stores. The credentials, links and backup targets it
holds, and the stores each one reaches (one hop minimum).

## 2. Entry
The realistic ways an attacker gets control: stolen device, stolen token, malicious dependency,
exposed listener, phishing. Rank by likelihood.

## 3. What the attacker gets
Reads, writes and pivots, per data class. The worst realistic outcome in one sentence.

## 4. Detection
The signal that would show this compromise (doctor check, backup anomaly, GitHub audit log,
login alert). Say whether the signal exists today.

## 5. Response
Containment steps in order: revoke the device (`devices` revoke route), rotate the named
credentials, sign out sessions, restore from backup. Point at an existing runbook if one
exists. Do not copy it.

## 6. Residual risk
What stays exposed after the response is complete.

## 7. Register entries
The ids added to `register.md`, with L, I and score.
```

## Scoring evidence

For each score, write one line that names its evidence: a command output, a file and line, or the
operator's statement. A score without evidence is `unverified`. Mark it that way in the register.
