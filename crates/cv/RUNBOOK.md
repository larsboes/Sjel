# CV builder runbook

This was `Packs/cv/skills/cv-builder/SKILL.md` until 2026-10-24. The Pack is gone; the runbook is
what a caller needs, and the `cv` CLI in this directory is the only interface either way. Keep it
here, beside the tool it drives, rather than in an agent skill that says the same thing.

The dashboard page `/cv` runs the same verbs.


`$CV` is the only interface.

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
5. Write every change through `$CV model get | jq '<change>' | $CV model put` (typed JSON) or
   `$CV edit get | <change> | $CV edit put` (raw YAML). Both refuse what does not parse or
   render, and commit the change in the overlay's git.
6. Record where a new fact comes from in the item's `source` field. YAML comments do not survive
   a form save.

## Track applications

- Run `$CV app list` to show all applications.
- Run `$CV app status <slug> <status> "<note>"` when the user reports a change.
  Status is one of: draft, applied, interview, offer, accepted, rejected, withdrawn.

## LinkedIn

LinkedIn has no write API. The user changes the profile by hand. Two sources fill the same
snapshot: the Member Snapshot API (live, needs a token) and a "Get a copy of your data" CSV
export (a manual download from LinkedIn, no token).

1. Prefer `$CV linkedin import <dir>`, where `<dir>` is the extracted export. It needs no token
   and no developer app. Otherwise run `$CV linkedin pull`, which reads the token from the
   keychain item `sjel-linkedin-dma`. If the token is missing or expired (HTTP 401), tell the
   user to create one: LinkedIn developer portal, OAuth token generator, scope
   `r_dma_portability_self_serve`. Then the user stores it with
   `security add-generic-password -U -s sjel-linkedin-dma -a "$USER" -w`.
   Do not ask for the token in the chat. A CSV export carries no About text, so `diff` reports
   About as absent rather than as different.
2. Run `$CV linkedin diff`. Report the differences in headline, positions and skills. Its second
   line names the snapshot's date and whether it came from the API or an export.
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
