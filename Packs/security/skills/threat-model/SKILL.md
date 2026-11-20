---
name: threat-model
description: Models threats against the estate this operator runs. It maps which capabilities, devices, accounts and stores hold each Sjel data class. It walks what one compromised asset exposes and reaches, and how the operator detects and contains it. It keeps a risk register with likelihood × impact scores and review dates in the private overlay. Use when asked to threat model Sjel or a device, "what if X gets hacked", blast radius, a stolen or lost phone or laptop, where sensitive data lives, a risk register or risk review, or after a security alert that names a credential or device. Do not use for vetting third-party software before it runs (use vendor-audit), handling a credential value (use secrets-guard), or active scanning or exploitation.
allowed-tools: Bash, Read, Grep, Glob, Write, Edit
---

# Threat model

Answer one question per run: **if this asset is compromised, what does the attacker get, and how
does the operator find out.** Model only. Never scan, probe or exploit anything.

## Where results go

Write every artifact to `<overlay>/data/security/threat-model/`. Resolve `<overlay>` from
`SJEL_OVERLAY_ROOT`, then `SJEL_PERSONAL_ROOT`. If neither is set, stop and say so.

- `estate.md` — the data map (step 1).
- `scenarios/<asset>.md` — one file per compromise scenario (step 2).
- `register.md` — the risk register (step 3).

Never write instance facts (hosts, device names, account names, paths) into the Sjel repository.
Sjel is public. Name a credential by its keychain entry or environment variable, never by value.

## 1. Map the estate

Build the inventory from live sources. Do not use a remembered inventory.

| Asset kind | Source |
|---|---|
| Capabilities | `sjel capability list`, then `tools/sjel-context with <capability>` for `requires`, `links_to`, `backup_target`, `proxy_api_only` |
| Devices | the devices capability (`GET /api/devices`), plus the operator's list of unpaired devices |
| Credentials | keychain entry names and overlay config keys. Read names only. Follow `secrets-guard`. |
| Accounts and remotes | `gh auth status`, `gh repo list --json nameWithOwner,visibility`, backup targets from `sjel backup status` |

Tag each asset with the highest data class it holds or can reach. The classes are `c0` Public,
`c1` Mine, `c2` Others and `c3` Secret (`docs/architecture/security.md`, "Agent access policy").
List an asset you cannot classify from evidence as `unclassified`. Unclassified is not clean.

## 2. Walk one compromise

Pick one asset. If the request names none and the choice is not clear, ask which one.
Fill the template in `references/scenario.md`. Two rules decide the impact score:

1. **Reach counts, not only storage.** An asset with no data of its own inherits the class of
   every store its credentials or links reach. Walk at least one hop of trust.
2. **Concentration is its own risk.** One asset that holds credentials for two separate trust
   domains collapses a boundary. Record it as a separate register entry. The fix is to split
   the credentials, not to rotate them.

Name what the inventory cannot see: browser sessions, OAuth grants held by the vendor, SSH keys,
the password manager, and synced iCloud data. These are part of the scenario, not an omission.

## 3. Keep the register

`register.md` holds one table. Columns: `id`, `risk`, `assets`, `class`, `L`, `I`, `score`,
`band`, `mitigation`, `owner`, `review_by`, `status`.

- `score = L × I`, each 1–5. Bands: Low 1–4, Medium 5–9, High 10–14, Critical 15–25.
- Impact anchors: 5 = `c3` or credentials to many systems. 4 = credentials to one data-bearing
  system, or a `c2` store. 3 = `c1` only, or write access to a public surface. 2 = limited `c1`.
  1 = `c0` only and no further hop.
- Likelihood anchors: public reach, the authentication boundary, patch state and credential
  hygiene. Cite the evidence for each score in the scenario file.
- Every entry gets a `review_by` date. A risk review lists overdue entries first.
- `status` is one of `open`, `mitigated`, `accepted`, `closed`. An `accepted` entry names who
  accepted it and the date.

## 4. Report

Lead with the worst realistic outcome and whether a detection signal exists today. Then list
the register entries this run added or changed, with id and score. Mark each Critical entry for
action now. When a mitigation is a Sjel code change, propose a canonical task
(`tools/work`). Do not change code from a threat-model run.

## Gaps this skill fills

`docs/architecture/security.md` ("Not covered") states that device loss is not in the threat
model. A lost-phone or lost-laptop scenario is the first run to do.
