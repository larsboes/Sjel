# archive

The archive holds unsorted source material awaiting import into its owning capabilities. Its
backup preserves the files independently of the knowledge vault and does not mark them imported.

Enable this data capability and set `[capability.archive] backup_source_root` in the machine
configuration to the absolute parent directory of `archive`. The source must exist. The backup
contains the `archive/` directory and uses the configured `backup-target`. Run
`tools/backup.sh archive`, then use `tools/restore.sh` to verify the resulting archive in isolation.
For a one-off streamed backup before configuring the capability, pass `--stream --source-root
<absolute-path>`; this does not write a backup receipt.
Keep source files until each import batch and its attachments have been verified.
