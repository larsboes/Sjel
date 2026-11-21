---
id: "gpui-note-rename"
title: "Make note rename and saves lossless"
state: "done"
area: "gpui"
kind: "fix"
priority: 1
spec: "docs/architecture/native-ui-boundaries.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Make note rename and saves lossless

## Outcome

Make note rename and saves lossless through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Preserve or prevent typing during an in-flight rename; compare-and-write stale bases atomically through the owner store, not separate reads/writes.
- Verify Enter moves the file and rewrites wikilinks while Esc/blur revert; include non-round-tripping and hub notes.
- Ask about hub folder semantics before changing them; review the missing list-row rename affordance separately from title rename.
- Run iOS build and paired-device regression; phone titles remain read-only while replica identity is path-keyed.

## Evidence

- `NoteEditor::can_edit` prevents typing during an in-flight rename (`!self.renaming`) after `request_rename` flushes pending edits; `Projects::note_title` binds `Escape` and `Blur` to `cancel_title`, and keeps phone titles read-only (`self.density != Density::Mobile && !self.repo.is_replica()`).
- `sjel-projects` (`ops::rename`) moves records atomically and rewrites wikilinks via `save_checked` compare-and-write, keeping hub note folders intact (`a_hub_note_rename_keeps_its_project_folder_and_rewrites_links`).
- `cargo test -p sjel-projects` and `cargo check -p sjel-gpui-ios --target aarch64-apple-ios-sim` pass.
- Live Mac, demo node (`tools/demo-shots app` with `SJEL_GPUI_KEYS` clicks): Enter renamed the hub note
  `Rename Lab` to `Lab Overview` inside its folder and rewrote `[[Rename Lab]]` in a body and in
  `projects:` frontmatter; the project kept its 3 notes. Enter renamed `Alpha` to `Alpha: Draft? #2`
  and moved 1 link. `Alpha: Draft/2?` was refused with a toast and no file changed. Blur reverted.
- Typing into the title lost every letter the Projects page binds (`b t f n j k l e`, space, `1`–`5`,
  `[`, `]`). Those bindings now use `Projects && !Input` (`apps/gpui/ui/src/projects/actions.rs`);
  `the_note_title_field_takes_letters_the_page_binds` fails without the change.
- Paired iPhone (`interceptor ios`, builds 2330 and 2357 from `ios/run-device`): a tap on an open note's title opens no field.
- Esc reverts in the app. The workspace's window-wide `escape` skipped no field and won over the
  title's own; it now binds under `!Terminal && !(Projects > Input)` (`apps/gpui/src/workspace/mod.rs`),
  so the finder keeps it. The same test covers it with an equivalent outer binding.
- `check_name` refuses `#`, `|`, `[` and `]`, which a wikilink reads as a heading, an alias or its end
  (`crates/projects/src/record.rs`).

- A hub rename keeps the project folder, so the project keeps its name while the hub note takes
  the new title. The folder semantics stay unchanged.
- The list-row rename affordance is not part of this task.
