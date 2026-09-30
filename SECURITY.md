# Security policy

## Supported versions

Sjel has no stable release line yet. Security fixes target the current `main` branch. Older
commits and archived branches are not supported. Neither are private deployment overlays or
downstream forks.

## Report a vulnerability privately

Use GitHub's **Report a vulnerability** flow on the repository Security page. Name the affected
commit or path and explain the impact. Add reproduction conditions and the smallest safe proof you
can provide.

Do not open a public issue with vulnerability details or credentials. Private hostnames and
addresses belong in the same channel. So do logs or screenshots that reveal deployment data. If
GitHub's reporting flow is unavailable, open a public issue that asks for a private contact route
and contains no details about the vulnerability.

The maintainer will acknowledge a usable report, confirm the disclosure channel, and coordinate a
fix and publication timeline when the finding is accepted. Please do not test against systems or
data you do not own or have permission to assess.

## Scope

Reports about Sjel's public code and schemas are in scope, as are its build and installer. Shipped
defaults and capability contracts are also covered. Findings that concern only a private overlay
or one deployment still belong in the private reporting channel, but they may be redirected if no
reusable Sjel defect exists.
