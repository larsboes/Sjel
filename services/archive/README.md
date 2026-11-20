# archive

The archive holds unsorted source material awaiting import into its owning capabilities. Its
backup preserves the files independently of the knowledge vault and does not mark them imported.

Enable this data capability and set `[capability.archive] backup_source_root` in the machine
configuration to the absolute parent directory of `archive`. Set the capability's `backup_target`
override to a destination on a separate physical device from the source. Run `tools/backup.sh archive`,
then use `tools/restore.sh` with the backup receipt to verify it in isolation. Pass
`--compare-archive-source <archive-dir>` to compare file paths, bytes and link targets with the source.

The archive includes the `archive/` tree and `archive-symlinks.json`. The JSON maps paths relative
to `archive/` to their original link targets. Targets are recorded without following them. Review
the targets before recreating any links after a restore.

For a one-off streamed backup before configuring the capability, pass `--stream --source-root
<absolute-path>`; this does not write a backup receipt.
Keep source files until each import batch and its attachments have been verified.
