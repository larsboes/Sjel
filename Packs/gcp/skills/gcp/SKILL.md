---
name: gcp
description: Guides safe discovery and use of Google Cloud CLI tools. Use when operating GCP projects, services, or Vertex AI through `gcloud`. Do not use for cloud-provider-neutral CLI discovery; use the system-tools skill.
---

# GCP

## Discover commands

Use the installed CLI's help as the source of valid commands and flags. Do not infer a command from an old example.

```bash
gcloud --help
gcloud <service> --help
gcloud <service> <command> --help
```

For a task involving a local helper, check that the command exists and read its own `--help` before using it. Do not assume helper names or paths.

## Check the active identity and project

Before a cloud operation, distinguish the active gcloud account, the configured project, and Application Default Credentials (ADC):

```bash
gcloud auth list
gcloud config get-value project
```

Use the project explicitly supplied by the user or by the current task. Do not guess a project ID or change the global project as a side effect of a read. Add `--project` to a command when its target must be explicit.

Never print, paste, or log access tokens or credential-file contents. To initialize user ADC when explicitly required:

```bash
gcloud auth application-default login
```

Do not run interactive login or change account/project configuration unless the user asked for it.

## Vertex AI

Discover the exact operation and model support from the installed CLI:

```bash
gcloud ai --help
gcloud ai models --help
```

Model availability, regions, quotas, and enablement vary by project. Use the selected project and service help; do not hard-code a person's project, account, model catalog, or regional assumptions into this skill.
