//! Top-level render dispatch for Projects: hubs, all tasks, all notes, and project detail.

use super::*;

impl Projects {
    pub(super) fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let cols = self.columns();
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        // 1. Note editor open with NO project selected (full-page on Mobile or outside All Notes;
        //    desktop All Notes renders a split-pane workspace below).
        if self.project.is_none()
            && (self.density == Density::Mobile || self.tab != ProjectsTab::AllNotes)
        {
            if let Some(editor) = &self.editor {
                let title = editor.read(cx).title.clone();
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .w_full()
                    .max_w(px(840.))
                    .mx_auto()
                    .pt_2()
                    .pb_6()
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(back_button(
                                        "‹ All Notes",
                                        cx.listener(|view, _, _, cx| view.close_note(cx)),
                                        palette,
                                    ))
                                    .child(
                                        div().text_xs().text_color(rgb(palette.muted)).child("/"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.muted))
                                            .child(title.clone()),
                                    ),
                            )
                            .child(self.note_title(editor, palette, cx))
                            .child(div().flex_1().min_h_0().child(editor.clone())),
                    )
                    .into_any_element();
            }
        }
        if self.project.is_some() {
            let hub_to_open = if self.project_section == ProjectSection::Note
                && self.editor.is_none()
                && self.opening.is_none()
            {
                self.project.as_ref().and_then(|(name, records)| {
                    records.value.as_ref().and_then(|r| {
                        r.iter()
                            .find(|t| t.kind == "project" || t.path == format!("{name}/{name}.md"))
                            .map(|t| t.path.clone())
                            .filter(|path| self.failed.as_ref() != Some(path))
                    })
                })
            } else {
                None
            };
            if let Some(hub_path) = hub_to_open {
                self.open_note(hub_path, true, window, cx);
            }

            let new_task = self.new_task_field(window, cx);
            let new_note = self.new_note_field(window, cx);
            let Some((name, records)) = &self.project else {
                return div().into_any_element();
            };
            let project_name = name.clone();

            let (open_tasks, supporting_notes_count) = records
                .value
                .as_ref()
                .map(|r| {
                    let hub_path = format!("{project_name}/{project_name}.md");
                    let open = r.iter().filter(|t| t.kind == "task" && !t.done).count();
                    let notes = r
                        .iter()
                        .filter(|t| t.kind != "task" && t.kind != "project" && t.path != hub_path)
                        .count();
                    (open, notes)
                })
                .unwrap_or((0, 0));

            let sec_entity = cx.entity();

            // Project header bar with breadcrumb and section tabs
            let project_header = div()
                .flex()
                .items_center()
                .justify_between()
                .when(self.density == Density::Mobile, |el| {
                    el.flex_col().items_start().gap_2()
                })
                .mb_2()
                .child(
                    div()
                        .id("project-breadcrumb-host")
                        .debug_selector(|| "project-breadcrumb-host".to_owned())
                        .child({
                            let owner = cx.entity();
                            crate::ely::navigation::Breadcrumb::new(
                                "project-breadcrumb",
                                [
                                    crate::ely::navigation::Crumb::new("projects", "Projects"),
                                    crate::ely::navigation::Crumb::new(
                                        "current",
                                        project_name.clone(),
                                    ),
                                ],
                            )
                            .on_select(move |level, _, _, cx| {
                                if level == 0 {
                                    owner.update(cx, |view, cx| view.close_project(cx));
                                }
                            })
                        }),
                )
                .child(sub_tabs(
                    "project-section-tabs",
                    vec![
                        (
                            "Project Note",
                            None,
                            self.project_section == ProjectSection::Note,
                        ),
                        (
                            "Tasks",
                            Some(open_tasks),
                            self.project_section == ProjectSection::Tasks,
                        ),
                        (
                            "Notes",
                            Some(supporting_notes_count),
                            self.project_section == ProjectSection::Notes,
                        ),
                        (
                            "Mail review",
                            None,
                            self.project_section == ProjectSection::MailReview,
                        ),
                        (
                            "Inventory links",
                            None,
                            self.project_section == ProjectSection::InventoryLinks,
                        ),
                    ],
                    move |ix, _, cx| {
                        sec_entity.update(cx, |view, cx| {
                            let target_sec = match ix {
                                1 => ProjectSection::Tasks,
                                2 => ProjectSection::Notes,
                                3 => ProjectSection::MailReview,
                                4 => ProjectSection::InventoryLinks,
                                _ => ProjectSection::Note,
                            };
                            if view.project_section != target_sec {
                                view.put_away_editor(cx);
                                view.project_section = target_sec;
                                if target_sec == ProjectSection::MailReview {
                                    view.load_project_mail(false, cx);
                                }
                                cx.notify();
                            }
                        });
                    },
                ));

            let content: AnyElement = match self.project_section {
                ProjectSection::Note => {
                    let has_hub = records.value.as_ref().is_some_and(|r| {
                        r.iter().any(|t| {
                            t.kind == "project"
                                || t.path == format!("{project_name}/{project_name}.md")
                        })
                    });

                    if records.loading || self.opening.is_some() {
                        placeholder(240.).into_any_element()
                    } else if let Some(editor) = &self.editor {
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .w_full()
                            .max_w(px(840.))
                            .mx_auto()
                            .pt_2()
                            .pb_6()
                            .child(
                                div()
                                    .size_full()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(self.note_title(editor, palette, cx))
                                    .when_some(
                                        self.promote_checklist_strip(
                                            &project_name,
                                            editor,
                                            palette,
                                            cx,
                                        ),
                                        |el, strip| el.child(strip),
                                    )
                                    .child(div().flex_1().min_h_0().child(editor.clone())),
                            )
                            .into_any_element()
                    } else if !has_hub {
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .py_16()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("No hub note found for {project_name}.")),
                            )
                            .child(
                                div()
                                    .id("create-project-note-btn")
                                    .px_4()
                                    .py_2()
                                    .rounded(px(6.))
                                    .bg(rgb(palette.accent))
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(0xffffff))
                                    .cursor_pointer()
                                    .hover(move |s| s.opacity(0.9))
                                    .on_click(cx.listener({
                                        let proj = project_name.clone();
                                        move |view, _, window, cx| {
                                            view.create_project_note(proj.clone(), window, cx);
                                        }
                                    }))
                                    .child(format!("+ Create {project_name}.md")),
                            )
                            .into_any_element()
                    } else {
                        placeholder(240.).into_any_element()
                    }
                }
                ProjectSection::MailReview => self.render_project_mail(records, palette, cx),
                ProjectSection::InventoryLinks => self.render_inventory_links(records, palette, cx),
                ProjectSection::Tasks => match (&records.value, &records.error) {
                    (_, Some(error)) => note(error.to_string(), palette.red).into_any_element(),
                    (None, None) => placeholder(240.).into_any_element(),
                    (Some(records_list), None) => {
                        let toolbar = self.database_toolbar(records_list, palette, cx);
                        let content = match self.task_view_mode {
                            TaskViewMode::Table => {
                                self.task_table(records_list, &new_task, palette, cx)
                            }
                            TaskViewMode::Board => {
                                self.task_board(records_list, &new_task, palette, cx)
                            }
                        };

                        let selected = self.task_panel.as_ref().and_then(|panel| {
                            records_list
                                .iter()
                                .find(|t| t.path == panel.path)
                                .map(|rec| (panel, tables::project_record_to_global_task(rec)))
                        });

                        let inner = match selected {
                            Some((panel, task)) if self.density == Density::Mobile => {
                                self.task_panel_view(panel, &task, palette, cx)
                            }
                            Some((panel, task)) => div()
                                .flex()
                                .items_start()
                                .gap_3()
                                .child(div().flex_1().min_w_0().child(content))
                                .child(self.task_panel_view(panel, &task, palette, cx))
                                .into_any_element(),
                            None => content,
                        };

                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .size_full()
                            .child(toolbar)
                            .child(inner)
                            .into_any_element()
                    }
                },
                ProjectSection::Notes => {
                    if let Some(editor) = &self.editor {
                        let active_path = editor.read(cx).path.clone();
                        let hub_path = format!("{project_name}/{project_name}.md");
                        let rail = (self.density != Density::Mobile)
                            .then(|| {
                                records.value.as_deref().map(|list| {
                                    let items: Vec<AnyElement> = list
                                        .iter()
                                        .filter(|rec| {
                                            rec.kind != "task"
                                                && rec.kind != "project"
                                                && rec.path != hub_path
                                        })
                                        .map(|rec| {
                                            let path = rec.path.clone();
                                            let is_current = path == active_path;
                                            div()
                                                .id(SharedString::from(format!(
                                                    "notes-rail-{}",
                                                    rec.path
                                                )))
                                                .px_2()
                                                .py_1p5()
                                                .rounded(px(6.))
                                                .cursor_pointer()
                                                .text_xs()
                                                .truncate()
                                                .when(is_current, |el| {
                                                    el.bg(rgba(tint(palette.accent, 0x26)))
                                                        .text_color(rgb(palette.accent))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                })
                                                .when(!is_current, |el| {
                                                    el.text_color(rgb(palette.text))
                                                        .hover(move |s| s.bg(rgba(palette.raised)))
                                                })
                                                .on_click(cx.listener(
                                                    move |view, _, window, cx| {
                                                        view.open_note(
                                                            path.clone(),
                                                            false,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                ))
                                                .child(rec.title.clone())
                                                .into_any_element()
                                        })
                                        .collect();
                                    div()
                                        .w(px(220.))
                                        .flex_none()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .pr_3()
                                        .border_r_1()
                                        .border_color(rgba(palette.border))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(palette.muted))
                                                .mb_1()
                                                .child("NOTES"),
                                        )
                                        .children(items)
                                })
                            })
                            .flatten();

                        let editor_pane = div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(back_button(
                                        format!("‹ {project_name} Notes"),
                                        cx.listener(|view, _, _, cx| {
                                            view.put_away_editor(cx);
                                            cx.notify();
                                        }),
                                        palette,
                                    ))
                                    .child(
                                        div().text_xs().text_color(rgb(palette.muted)).child("/"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.muted))
                                            .child(editor.read(cx).title.clone()),
                                    ),
                            )
                            .child(self.note_title(editor, palette, cx))
                            .when_some(
                                self.promote_checklist_strip(&project_name, editor, palette, cx),
                                |el, strip| el.child(strip),
                            )
                            .child(div().flex_1().min_h_0().child(editor.clone()));

                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .gap_4()
                            .w_full()
                            .pt_2()
                            .pb_6()
                            .when_some(rail, |el, r| el.child(r))
                            .child(editor_pane)
                            .into_any_element()
                    } else {
                        match (&records.value, &records.error) {
                            (_, Some(error)) => {
                                note(error.to_string(), palette.red).into_any_element()
                            }
                            (None, None) => placeholder(240.).into_any_element(),
                            (Some(records_list), None) => {
                                let hub_path = format!("{project_name}/{project_name}.md");
                                let note_records: Vec<_> = records_list
                                    .iter()
                                    .filter(|record| {
                                        record.kind != "task"
                                            && record.kind != "project"
                                            && record.path != hub_path
                                    })
                                    .collect();

                                let columns = vec![
                                    Column::new("title", "NOTE").min_width(rems(14.)),
                                    Column::new("preview", "PREVIEW").min_width(rems(16.)),
                                    Column::new("kind", "TYPE").width(rems(6.)),
                                    Column::new("actions", "")
                                        .width(rems(6.))
                                        .actions([RowAction::new("delete", "Delete")]),
                                ];
                                let rows: Rc<Vec<Row>> = Rc::new(
                                    note_records
                                        .iter()
                                        .map(|record| {
                                            let snippet = extract_note_snippet(&record.content)
                                                .unwrap_or_else(|| "—".into());
                                            Row::new(
                                                record.path.clone(),
                                                [
                                                    Cell::Text(record.title.clone().into()),
                                                    Cell::Text(snippet.into()),
                                                    Cell::Tag(
                                                        record.kind.clone().into(),
                                                        Tone::Neutral,
                                                    ),
                                                    Cell::Actions(vec!["delete".into()]),
                                                ],
                                            )
                                        })
                                        .collect(),
                                );
                                let (on_open, on_act) = (cx.entity(), cx.entity());
                                let notes_content =
                                    DataTable::new("project-supporting-notes-table", columns)
                                        .label("Supporting notes")
                                        .rows(rows)
                                        .on_open(move |key, window, cx| {
                                            let path = key.to_string();
                                            on_open.update(cx, |view, cx| {
                                                view.open_note(path, true, window, cx);
                                            });
                                        })
                                        .on_action(move |key, action, window, cx| {
                                            if action.as_ref() == "delete" {
                                                let path = key.to_string();
                                                on_act.update(cx, |view, cx| {
                                                    view.remove_record(path, window, cx);
                                                });
                                            }
                                        })
                                        .empty("No supporting notes yet in this project.")
                                        .into_any_element();

                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(rgb(palette.muted))
                                                    .child(format!(
                                                        "Supporting Notes ({supporting_notes_count})"
                                                    )),
                                            )
                                            .child(
                                                crate::ely::buttons::Button::new(
                                                    "add-note-btn",
                                                    "+ Add note",
                                                )
                                                .size(crate::ely::theme::ControlSize::Sm)
                                                .variant(
                                                    crate::ely::buttons::ButtonVariant::Secondary,
                                                )
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    view.adding_note = !view.adding_note;
                                                    cx.notify();
                                                })),
                                            ),
                                    )
                                    .when(self.adding_note, |sec| {
                                        sec.child(
                                            div()
                                                .p_2()
                                                .rounded(px(8.))
                                                .bg(rgba(palette.raised))
                                                .child(Input::new(&new_note).small()),
                                        )
                                    })
                                    .child(notes_content)
                                    .into_any_element()
                            }
                        }
                    }
                }
            };

            let summary_strip = records.value.as_deref().map(|r| {
                let mut open = 0usize;
                let mut blocked = 0usize;
                let mut done = 0usize;
                let mut next_due: Option<&str> = None;
                for rec in r.iter().filter(|rec| rec.kind == "task") {
                    let status = tables::parse_frontmatter_status(&rec.content, rec.done);
                    if status == "done" || status == "cancelled" {
                        done += 1;
                    } else {
                        open += 1;
                        if status == "blocked" || status == "waiting" {
                            blocked += 1;
                        }
                        if let Some(d) = rec.due.as_deref().filter(|d| !d.is_empty()) {
                            if next_due.is_none_or(|cur| d < cur) {
                                next_due = Some(d);
                            }
                        }
                    }
                }
                let total = open + done;
                let ratio = if total > 0 {
                    done as f32 / total as f32
                } else {
                    0.0
                };
                let mail_count = self.mail.value.as_ref().map(|m| m.len());
                let links_count = self.links.value.as_ref().map(|l| l.len());
                let next_due_label = next_due.map(str::to_string);

                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(tint(palette.raised, 0x55)))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(div().w(px(110.)).child(crate::primitives::progress_bar(
                                ratio,
                                palette.green,
                                palette,
                            )))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.text))
                                    .child(format!("{open} open · {done} done")),
                            )
                            .when(blocked > 0, |el| {
                                el.child(
                                    crate::ely::data_display::Badge::new(format!(
                                        "{blocked} blocked"
                                    ))
                                    .tone(Tone::Warning),
                                )
                            })
                            .when_some(next_due_label, |el, due| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("Next due {due}")),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                crate::ely::buttons::Button::new(
                                    "project-touch-notes",
                                    if supporting_notes_count > 0 {
                                        format!("Notes ({supporting_notes_count})")
                                    } else {
                                        "Notes".to_string()
                                    },
                                )
                                .size(crate::ely::theme::ControlSize::Sm)
                                .variant(crate::ely::buttons::ButtonVariant::Ghost)
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.put_away_editor(cx);
                                    view.project_section = ProjectSection::Notes;
                                    cx.notify();
                                })),
                            )
                            .child(
                                crate::ely::buttons::Button::new(
                                    "project-touch-mail",
                                    match mail_count {
                                        Some(n) if n > 0 => format!("Mail ({n})"),
                                        _ => "Mail".to_string(),
                                    },
                                )
                                .size(crate::ely::theme::ControlSize::Sm)
                                .variant(crate::ely::buttons::ButtonVariant::Ghost)
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::MailReview;
                                        view.load_project_mail(false, cx);
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                crate::ely::buttons::Button::new(
                                    "project-touch-inventory",
                                    match links_count {
                                        Some(n) if n > 0 => format!("Inventory ({n})"),
                                        _ => "Inventory".to_string(),
                                    },
                                )
                                .size(crate::ely::theme::ControlSize::Sm)
                                .variant(crate::ely::buttons::ButtonVariant::Ghost)
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::InventoryLinks;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                crate::ely::buttons::Button::new(
                                    "project-primary-add-task",
                                    "+ Task",
                                )
                                .size(crate::ely::theme::ControlSize::Sm)
                                .variant(crate::ely::buttons::ButtonVariant::Primary)
                                .on_click(cx.listener(
                                    |view, _, _, cx| {
                                        view.put_away_editor(cx);
                                        view.project_section = ProjectSection::Tasks;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
            });

            return div()
                .id("records")
                .size_full()
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.scroll)
                .flex()
                .flex_col()
                .gap_3()
                .child(project_header)
                .children(summary_strip)
                .child(content)
                .into_any_element();
        }

        if self.review_mode && self.project.is_none() {
            return self.review_view(palette, cx);
        }

        // Top navigation tabs: rendered inside the page only when mounted standalone or on mobile
        // (on Desktop, `header.rs` renders the unified tab strip in the window header).
        let show_inline_tabs = self.density == Density::Mobile || cfg!(test);
        let projects_count = self
            .projects
            .value
            .as_ref()
            .map(|list| list.iter().filter(|p| !p.archived).count())
            .filter(|&c| c > 0);
        let tasks_count = self
            .all_tasks
            .value
            .as_ref()
            .map(Vec::len)
            .filter(|&c| c > 0);
        let notes_count = self
            .all_notes
            .value
            .as_ref()
            .map(Vec::len)
            .filter(|&c| c > 0);
        let tab_entity = cx.entity();
        let tab_bar = show_inline_tabs.then(|| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb_3()
                .child(sub_tabs(
                    "projects-main-tabs",
                    vec![
                        ("Projects", projects_count, self.tab == ProjectsTab::Hubs),
                        ("All Tasks", tasks_count, self.tab == ProjectsTab::AllTasks),
                        ("All Notes", notes_count, self.tab == ProjectsTab::AllNotes),
                    ],
                    move |ix, _, cx| {
                        tab_entity.update(cx, |view, cx| {
                            view.tab = match ix {
                                1 => ProjectsTab::AllTasks,
                                2 => ProjectsTab::AllNotes,
                                _ => ProjectsTab::Hubs,
                            };
                            cx.notify();
                        });
                    },
                ))
        });

        if self.tab == ProjectsTab::AllNotes {
            let search_input = self.notes_search_input_field(window, cx);
            let list_body = match (&self.all_notes.value, &self.all_notes.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(320.),
                (Some(notes), None) => self.all_notes_table(notes, &search_input, cx),
            };
            if let Some(editor) = self.editor.clone() {
                let open_path = editor.read(cx).path().to_owned();
                let title = editor.read(cx).title.clone();
                let related: Vec<(String, String)> = self
                    .all_notes
                    .value
                    .as_ref()
                    .and_then(|notes| {
                        notes
                            .iter()
                            .find(|n| n.path == open_path)
                            .map(|current| (current.project.clone(), notes))
                    })
                    .map(|(proj, notes)| {
                        notes
                            .iter()
                            .filter(|n| n.project == proj && n.path != open_path)
                            .take(6)
                            .map(|n| (n.path.clone(), n.title.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let editor_pane = crate::primitives::surface_card(palette)
                    .flex_1()
                    .min_w_0()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.muted))
                                    .child(title),
                            )
                            .child(back_button(
                                "Close note",
                                cx.listener(|view, _, _, cx| view.close_note(cx)),
                                palette,
                            )),
                    )
                    .child(self.note_title(&editor, palette, cx))
                    .child(div().flex_1().min_h_0().child(editor))
                    .when(!related.is_empty(), |pane| {
                        pane.child(
                            div()
                                .pt_2()
                                .border_t_1()
                                .border_color(rgba(palette.border))
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(palette.muted))
                                        .child("Related:"),
                                )
                                .children(related.into_iter().map(|(rel_path, rel_title)| {
                                    crate::ely::buttons::Button::new(
                                        SharedString::from(format!("related-note-{rel_path}")),
                                        rel_title,
                                    )
                                    .size(crate::ely::theme::ControlSize::Sm)
                                    .variant(crate::ely::buttons::ButtonVariant::Ghost)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.open_note(rel_path.clone(), true, window, cx);
                                    }))
                                })),
                        )
                    });
                return div()
                    .id("all-notes-split")
                    .debug_selector(|| "all-notes-split".into())
                    .size_full()
                    .flex()
                    .flex_col()
                    .children(tab_bar)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .items_start()
                            .gap_4()
                            .child(
                                div()
                                    .id("all-notes")
                                    .debug_selector(|| "all-notes".into())
                                    .w(px(360.))
                                    .flex_none()
                                    .h_full()
                                    .overflow_y_scroll()
                                    .child(section("Cross-Project Notes", list_body, palette)),
                            )
                            .child(editor_pane),
                    )
                    .into_any_element();
            }
            return div()
                .id("all-notes")
                .size_full()
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.scroll)
                .children(tab_bar)
                .child(section("Cross-Project Notes", list_body, palette))
                .into_any_element();
        }

        if self.tab == ProjectsTab::AllTasks {
            let search_input = self.search_input_field(window, cx);
            let show_charts = self.show_charts;
            let toggle_charts = cx.listener(|view, on: &bool, _, cx| {
                view.show_charts = *on;
                cx.notify();
            });
            let charts_toggle = crate::ely::buttons::ToggleButton::new(
                "toggle-tasks-charts",
                crate::ely::buttons::ToggleItem::new("charts").label("Charts"),
                show_charts,
            )
            .size(crate::ely::theme::ControlSize::Sm)
            .on_toggle(move |on, window, cx| toggle_charts(&on, window, cx));
            let body = match (&self.all_tasks.value, &self.all_tasks.error) {
                (_, Some(error)) => note(error.to_string(), palette.red),
                (None, None) => placeholder(320.),
                (Some(tasks), None) => {
                    let pressure = show_charts
                        .then(|| charts::tasks_pressure_charts(tasks, self.density, palette, cx))
                        .flatten();
                    let table = self.all_tasks_table(tasks, &search_input, palette, cx);
                    let selected = self.task_panel.as_ref().and_then(|panel| {
                        tasks
                            .iter()
                            .find(|t| t.id == panel.path)
                            .map(|task| (panel, task))
                    });
                    let content = match selected {
                        // A phone has no room beside the table: the panel takes its place.
                        Some((panel, task)) if self.density == Density::Mobile => {
                            self.task_panel_view(panel, task, palette, cx)
                        }
                        Some((panel, task)) => div()
                            .flex()
                            .items_start()
                            .gap_3()
                            .child(div().flex_1().min_w_0().child(table))
                            .child(self.task_panel_view(panel, task, palette, cx))
                            .into_any_element(),
                        None => table,
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().flex().justify_end().child(charts_toggle))
                        .when_some(pressure, |el, strip| el.child(strip))
                        .child(content)
                        .into_any_element()
                }
            };
            return div()
                .id("all-tasks")
                .size_full()
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .vertical_scrollbar(&self.scroll)
                .children(tab_bar)
                .child(section("Cross-Project Tasks", body, palette))
                .into_any_element();
        }

        // Projects Hub View
        let search_input = self.search_input_field(window, cx);
        let new_project = self.new_project_field(window, cx);

        let archived_count = self
            .projects
            .value
            .as_ref()
            .map(|list| list.iter().filter(|p| p.archived).count())
            .unwrap_or(0);

        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .mb_2()
            .child(search_box(cols.search).child(Input::new(&search_input).small()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child({
                        let toggle_charts = cx.listener(|view, on: &bool, _, cx| {
                            view.show_charts = *on;
                            cx.notify();
                        });
                        crate::ely::buttons::ToggleButton::new(
                            "toggle-hubs-charts",
                            crate::ely::buttons::ToggleItem::new("charts").label("Charts"),
                            self.show_charts,
                        )
                        .size(crate::ely::theme::ControlSize::Sm)
                        .on_toggle(move |on, window, cx| toggle_charts(&on, window, cx))
                    })
                    .child({
                        let toggle_archived = cx.listener(|view, on: &bool, _, cx| {
                            view.show_archived = *on;
                            cx.notify();
                        });
                        crate::ely::buttons::ToggleButton::new(
                            "toggle-archived",
                            crate::ely::buttons::ToggleItem::new("archived").label(
                                if self.show_archived {
                                    "Show Active".to_string()
                                } else if archived_count > 0 {
                                    format!("Archived ({archived_count})")
                                } else {
                                    "Archived".to_string()
                                },
                            ),
                            self.show_archived,
                        )
                        .size(crate::ely::theme::ControlSize::Sm)
                        .on_toggle(move |on, window, cx| toggle_archived(&on, window, cx))
                    })
                    .child(
                        crate::ely::buttons::Button::new("open-review-btn", "Guided Review")
                            .size(crate::ely::theme::ControlSize::Sm)
                            .variant(crate::ely::buttons::ButtonVariant::Ghost)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.review_mode = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        crate::ely::buttons::Button::new("new-project-btn", "+ New Project")
                            .size(crate::ely::theme::ControlSize::Sm)
                            .variant(crate::ely::buttons::ButtonVariant::Primary)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.adding_project = !view.adding_project;
                                cx.notify();
                            })),
                    ),
            );

        let body = match (&self.projects.value, &self.projects.error) {
            (_, Some(error)) => note(error.to_string(), palette.red),
            (None, None) => placeholder(320.),
            (Some(_), None) => {
                let filtered = self.filtered_projects();
                let portfolio = self
                    .show_charts
                    .then(|| charts::hubs_portfolio_charts(&filtered, self.density, palette, cx))
                    .flatten();
                let table = self.hubs_table(&filtered, palette, cx);
                div()
                    .flex()
                    .flex_col()
                    .when_some(portfolio, |el, charts_row| el.child(charts_row))
                    .child(table)
                    .into_any_element()
            }
        };

        div()
            .id("projects")
            .size_full()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .vertical_scrollbar(&self.scroll)
            .children(tab_bar)
            .child(toolbar)
            .when(self.adding_project, |p| {
                p.child(
                    div()
                        .mb_3()
                        .p_2()
                        .rounded(px(8.))
                        .bg(rgba(palette.raised))
                        .child(Input::new(&new_project).small()),
                )
            })
            .when_some(self.blocked_band(palette, cx), |page, band| {
                page.child(band)
            })
            .child(section("Projects", body, palette))
            .into_any_element()
    }
}

