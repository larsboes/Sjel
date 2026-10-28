# GCP

A harness-neutral guide for using `gcloud` without guessing commands or embedding a person's account and project IDs in shared instructions.

## Boundary

This pack contains generic CLI and authentication guidance only. Project catalogs, account details, local wrappers, and work-specific endpoints belong in the active private overlay.

## Activate

```sh
./tools/packs-pi deploy gcp
./tools/packs-claude deploy gcp
./tools/packs-codex deploy gcp
```

The pack is MIT-licensed. It uses Google Cloud CLI help and public product behavior as its source of truth; it does not vendor Google code or documentation.
