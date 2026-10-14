---
name: cv-builder
description: Builds CV PDFs from one profile-tagged master file, tracks job applications, compares the master with the LinkedIn profile, and gives career guidance from that data. All actions go through the local `cv` CLI. Use when the user wants a CV for a job, a company, an event or a language, wants to log or update an application, wants to update LinkedIn, or asks how their profile fits a job posting. Do not use for cover letters or other prose that is not CV content.
allowed-tools: Read, Bash, Edit
---

# CV Builder

`$CV` is the only interface. This skill is the runbook. The dashboard page `/cv` runs the same
verbs for the user.

## Variables

| var | value | notes |
|-----|-------|-------|
| `$CV` | `$SJEL_ROOT/capabilities/cv/cv` | launcher. `$CV` with no argument prints every verb. |
| `$DATA` | `$SJEL_PERSONAL_ROOT/data/cv` | personal data. Never copy it into the Sjel repo, a commit or this skill. |
| `$MASTER` | `$DATA/master_cv.yaml` | the one source of CV facts. Shape: `capabilities/cv/master_cv.schema.yaml`. |
| `$APPS` | `$DATA/applications/<slug>.yaml` | one file per application. It is also the tracker. |

## Build a CV

1. Run `$CV list-profiles`. Do not use a profile list from memory.
2. Select the profile that is nearest to the request. If two profiles are equally near, ask the user.
3. Select the language. A German posting or company gets `de`. All other requests get `en`.
4. Run `$CV build --profile <tag> --lang <en|de>`. Give the user the PDF path that it prints.

## Build a CV for one application or event

1. Run `$CV app new <slug> --company <name> --role <title> --profile <tag> --lang <en|de> --url <posting>`.
   The slug contains only lowercase letters, digits and dashes.
2. If the user gives a posting, read it. To include or exclude specific items, add their `id`s
   to `pin:` or `hide:` in the application file. Write `title` and `summary` overrides into the
   application file, each as `{en: ..., de: ...}`. Use only facts that are in `$MASTER`.
3. Write the fit into `notes`: which requirements the master covers, and which it does not.
4. Run `$CV app build <slug>`. Show the user the overrides and the PDF path.

## Curate the master

The master holds every fact. Focused profiles are one page. `general` is the full CV and the
reference for LinkedIn.

1. Add a new fact to `$MASTER` with `profiles:` tags and a `priority`. Do not delete old facts for space.
2. Run `$CV build --all`. Each line reports the page count and the priority level it kept.
3. If a build fails because priority-1 items do not fit, lower the priority of the least
   relevant item for that profile, e.g. `priority: {default: 1, hackathon: 2}`. Ask the user which item.
4. To reword bullets, edit the text in place. Git keeps the old wording.

## Track applications

- Run `$CV app list` to show all applications.
- Run `$CV app status <slug> <status> "<note>"` when the user reports a change.
  Status is one of: draft, applied, interview, offer, accepted, rejected, withdrawn.

## LinkedIn

LinkedIn has no write API. The user changes the profile by hand.

1. Run `$CV linkedin pull`. It reads the token from the keychain item `sjel-linkedin-dma`.
   If the token is missing or expired (HTTP 401), tell the user to create one: LinkedIn
   developer portal, OAuth token generator, scope `r_dma_portability_self_serve`. Then the
   user stores it with `security add-generic-password -U -s sjel-linkedin-dma -a "$USER" -w`.
   Do not ask for the token in the chat.
2. Run `$CV linkedin diff`. Report the differences in headline, positions and skills.
3. Run `$CV linkedin sheet --lang en`. Give the user the text to paste.

## Career guidance

Use `$MASTER`, `$DATA/linkedin/*.json` (if present) and the application files as the evidence.
Give advice that names the item it is based on. Examples: a skill that three postings ask for
and the master does not have, a role whose bullets have no measured result, or an application
with no change for 14 days.

## Rules

- **Facts come from `$MASTER`.** An application override can select and reword facts.
  It must not add a fact. If a fact is missing, propose the master edit and its `profiles:` tags.
  Edit `$MASTER` only after the user approves that edit.
- **An unknown profile tag fails the build.** This is intentional. Run `list-profiles` first.
- **`build --all` obeys `profile_langs`** in the master. So the output is not always profiles × 2.
- **Bold text in YAML prints its asterisks.** Use the `label` field of a section bullet for bold.
- **Experience and education dates are plain strings.** A German build shows them as written.

## Verify

Run `$SJEL_ROOT/capabilities/cv/check`. It runs every verb on a temporary overlay and prints
`check: ok`.
