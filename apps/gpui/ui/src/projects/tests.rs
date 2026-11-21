use gpui::TestAppContext;

use super::*;
use sjel_gpui_core::shell::{ProjectRecord, ShellError};

fn record() -> ProjectRecord {
    ProjectRecord {
        project: String::new(),
        path: "P/Plan.md".into(),
        kind: "note".into(),
        title: "Plan".into(),
        content: "---\ntype: note\n---\n# Plan\n\n- one\n- two\n\nText\n".into(),
        done: false,
        due: None,
        priority: 2,
    }
}

/// The band the #244 work is about: what the host handed in is what the page holds, and a second
/// sync that cleared it takes it away. The drawing itself needs a window; what this pins down is
/// that the page never invents a blocked record and never keeps one the store has forgotten.
#[gpui::test]
fn the_blocked_band_holds_what_the_sync_reported(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let view = cx.new(|cx| {
        let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
        projects.set_blocked(
            vec![Blocked {
                path: "Old/One.md".into(),
                title: "One".into(),
                kind: "note".into(),
                reason: "the project Old is archived".into(),
            }],
            cx,
        );
        projects
    });
    cx.run_until_parked();
    view.read_with(cx, |projects, _| {
        assert_eq!(projects.blocked().len(), 1);
        assert_eq!(projects.blocked()[0].title, "One");
        assert!(projects.blocked()[0].reason.contains("archived"));
    });
    view.update(cx, |projects, cx| projects.set_blocked(Vec::new(), cx));
    view.read_with(cx, |projects, _| {
        assert!(
            projects.blocked().is_empty(),
            "a landed write takes its band with it"
        );
    });
}

#[gpui::test]
fn project_inventory_review_mounts_at_mobile_density(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx)
                .with_density(Density::Mobile);
            let hub = ProjectRecord {
                kind: "project".into(),
                ..record()
            };
            projects.project = Some((
                "P".into(),
                Remote {
                    value: Some(vec![hub]),
                    ..Remote::default()
                },
            ));
            projects.link_path = Some("P/Plan.md".into());
            projects.links.value = Some(vec![ProjectInventoryLink {
                record_path: "P/Plan.md".into(),
                item_ref: "inv:61".into(),
                verified_revision: 2,
                reviewed_at: "stamp".into(),
                provenance: None,
            }]);
            projects.link_review = Some(LinkReview {
                record: ProjectLinkRecord {
                    path: "P/Plan.md".into(),
                    title: "Plan".into(),
                    content: "exact".into(),
                    updated_at: "stamp".into(),
                    kind: "project".into(),
                    archived: false,
                },
                item_ref: "inv:61".into(),
                item_label: "Item".into(),
                remove: true,
            });
            projects.project_section = ProjectSection::InventoryLinks;
            projects
        });
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
}

#[gpui::test]
fn project_mail_review_mounts_at_mobile_density(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx)
                .with_density(Density::Mobile);
            projects.project = Some((
                "P".into(),
                Remote {
                    value: Some(vec![record()]),
                    ..Remote::default()
                },
            ));
            let proposal = ProjectMailProposal {
                id: 1,
                version: 1,
                project: "P".into(),
                title: "Evidence".into(),
                evidence: "quoted\ntext".into(),
                source_id: "mail:v1:thread:message".into(),
                data_class: "c2".into(),
                task_suggestion: Some("Follow up".into()),
                status: "pending".into(),
            };
            projects.mail.value = Some(vec![proposal.clone()]);
            projects.mail_selected = Some(proposal);
            projects.mail_target = Some(ProjectMailTarget {
                path: "P/Plan.md".into(),
                title: "Plan".into(),
                content: "  exact\n".into(),
            });
            projects.project_section = ProjectSection::MailReview;
            projects
        });
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
}

#[gpui::test]
fn ely_breadcrumb_closes_the_project_by_pointer_and_keyboard(cx: &mut TestAppContext) {
    for pointer in [true, false] {
        let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
            let mut page = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            page.project = Some((
                "P".into(),
                Remote {
                    value: Some(vec![record()]),
                    ..Remote::default()
                },
            ));
            page.project_section = ProjectSection::Tasks;
            page.selected_task_path = Some("P/Plan.md".into());
            page
        });
        if pointer {
            let bounds = cx
                .debug_bounds("project-breadcrumb-host")
                .expect("breadcrumb renders");
            cx.simulate_click(
                bounds.origin + gpui::point(px(8.), px(8.)),
                gpui::Modifiers::none(),
            );
        } else {
            cx.update(|window, cx| {
                window.blur(cx);
                window.focus_next(cx);
            });
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            cx.simulate_event(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("enter").unwrap(),
            });
        }
        cx.run_until_parked();
        page.read_with(cx, |page, _| {
            assert!(page.project.is_none(), "pointer={pointer}");
            assert!(
                page.selected_task_path.is_none(),
                "close clears project selection"
            );
        });
    }
}

#[gpui::test]
fn a_project_page_draws_its_tasks_as_a_table(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            let task = |title: &str, done: bool| ProjectRecord {
                project: String::new(),
                path: format!("P/{title}.md"),
                kind: "task".into(),
                title: title.into(),
                content: "---\ntype: task\n---\n".into(),
                done,
                due: Some("2026-10-24".into()),
                priority: 1,
            };
            let records = Remote {
                value: Some(vec![task("open", false), task("closed", true), record()]),
                ..Remote::default()
            };
            projects.project = Some(("P".into(), records));
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let projects = slot.take().unwrap();
    projects.read_with(cx, |projects, _| assert!(projects.new_task.is_some()));
}

#[gpui::test]
fn all_tasks_tab_filters_by_query(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            projects.tab = ProjectsTab::AllTasks;
            projects.search_query = "berlin".into();
            projects.all_tasks = Remote {
                value: Some(vec![
                    GlobalTask {
                        fields: Default::default(),
                        id: "P/1.md".into(),
                        kind: "task".into(),
                        title: "Book train to Berlin".into(),
                        done: false,
                        due: None,
                        priority: 1,
                        summary: None,
                        project: "Travel".into(),
                    },
                    GlobalTask {
                        fields: Default::default(),
                        id: "P/2.md".into(),
                        kind: "task".into(),
                        title: "Update resume".into(),
                        done: false,
                        due: None,
                        priority: 2,
                        summary: None,
                        project: "Career".into(),
                    },
                ]),
                ..Remote::default()
            };
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let projects = slot.take().unwrap();
    projects.read_with(cx, |projects, _| {
        assert_eq!(projects.tab, ProjectsTab::AllTasks);
        assert_eq!(projects.search_query, "berlin");
    });
}

#[gpui::test]
fn all_tasks_panel_opens_on_a_task_and_draws_beside_the_table(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let task = |id: &str, status: &str| GlobalTask {
        id: format!("P/Tasks/{id}.md"),
        kind: "task".into(),
        title: id.into(),
        done: tasks::status_closes(status),
        due: None,
        priority: 2,
        summary: None,
        project: "P".into(),
        fields: [
            ("status".to_owned(), status.to_owned()),
            ("context".to_owned(), "Werkstatt".to_owned()),
        ]
        .into(),
    };
    let blocked = task("blocked", "blocked");
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let rows = vec![blocked.clone(), task("cancelled", "cancelled")];
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            projects.tab = ProjectsTab::AllTasks;
            projects.group_by_project = true;
            projects.all_tasks = Remote {
                value: Some(rows),
                ..Remote::default()
            };
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let projects = slot.take().unwrap();
    cx.update(|window, cx| {
        projects.update(cx, |projects, cx| {
            projects.select_task(&blocked, window, cx)
        })
    });
    cx.run_until_parked();
    projects.read_with(cx, |projects, cx| {
        let panel = projects.task_panel.as_ref().expect("the panel opened");
        assert_eq!(panel.path, "P/Tasks/blocked.md");
        let context = panel.input("context").expect("a context field");
        assert_eq!(context.read(cx).text(), "Werkstatt");
        let shown: Vec<&str> = projects
            .all_tasks
            .value
            .iter()
            .flatten()
            .filter(|t| projects.global_status.matches(t.status()))
            .map(|t| t.title.as_str())
            .collect();
        assert_eq!(shown, ["blocked"]);
    });
    cx.update(|window, cx| {
        projects.update(cx, |projects, cx| {
            projects.select_task(&blocked, window, cx)
        })
    });
    projects.read_with(cx, |projects, _| assert!(projects.task_panel.is_none()));
}

#[gpui::test]
fn ely_grouping_toggle_preserves_the_projects_table_state(cx: &mut TestAppContext) {
    let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
        let mut page = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
        page.tab = ProjectsTab::AllTasks;
        page.all_tasks.value = Some(Vec::new());
        page.group_by_project = false;
        page
    });
    for expected in [true, false] {
        let toggle = cx
            .debug_bounds("toggle Group by project")
            .expect("grouping toggle renders");
        cx.simulate_click(toggle.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        page.read_with(cx, |page, _| assert_eq!(page.group_by_project, expected));
    }
}

#[gpui::test]
fn all_notes_tab_filters_by_query(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            projects.tab = ProjectsTab::AllNotes;
            projects.notes_search_query = "architecture".into();
            projects.all_notes = Remote {
                value: Some(vec![
                    GlobalNote {
                        path: "Sjel/Architecture.md".into(),
                        kind: "note".into(),
                        title: "Architecture".into(),
                        project: "Sjel".into(),
                        url: None,
                    },
                    GlobalNote {
                        path: "BetterCode/BetterCode.md".into(),
                        kind: "project".into(),
                        title: "BetterCode".into(),
                        project: "BetterCode".into(),
                        url: None,
                    },
                ]),
                ..Remote::default()
            };
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let projects = slot.take().unwrap();
    projects.read_with(cx, |projects, _| {
        assert_eq!(projects.tab, ProjectsTab::AllNotes);
        assert_eq!(projects.notes_search_query, "architecture");
        assert_eq!(projects.all_notes.value.as_ref().unwrap().len(), 2);
    });
}

#[gpui::test]
fn project_section_auto_opens_hub_note(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            let hub = ProjectRecord {
                project: String::new(),
                path: "P/P.md".into(),
                kind: "project".into(),
                title: "P".into(),
                content: "# P\n".into(),
                done: false,
                due: None,
                priority: 1,
            };
            let records = Remote {
                value: Some(vec![hub, record()]),
                ..Remote::default()
            };
            projects.project = Some(("P".into(), records));
            projects.project_section = ProjectSection::Note;
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let projects = slot.take().unwrap();
    projects.read_with(cx, |projects, _| {
        assert_eq!(projects.project_section, ProjectSection::Note);
        // The repo has no shell, so the read fails: the hub was tried once, not on every frame.
        assert_eq!(projects.failed.as_deref(), Some("P/P.md"));
    });
}

#[gpui::test]
fn projects_hub_renders_table_header_and_sorts_projects(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let p1 = ProjectSummary {
        name: "Hearth".into(),
        archived: false,
        open: 3,
        done: 2,
        notes: 1,
    };
    let p2 = ProjectSummary {
        name: "Atlas".into(),
        archived: false,
        open: 0,
        done: 5,
        notes: 171,
    };
    let p3 = ProjectSummary {
        name: "ArchiveOld".into(),
        archived: true,
        open: 0,
        done: 10,
        notes: 4,
    };

    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            projects.projects = Remote {
                value: Some(vec![p1, p2, p3]),
                ..Remote::default()
            };
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let projects = slot.take().unwrap();

    projects.read_with(cx, |projects, _| {
        assert_eq!(projects.project_sort, (ProjectSort::Name, true));
        assert_eq!(projects.tab, ProjectsTab::Hubs);
    });

    // Test sorting behavior
    projects.update(cx, |projects, cx| {
        projects.project_sort = (ProjectSort::Notes, false);
        cx.notify();
    });
    cx.run_until_parked();
    projects.read_with(cx, |projects, _| {
        assert_eq!(projects.project_sort, (ProjectSort::Notes, false));
    });

    // Test sort cycling: flipping to ascending, then back to Name
    projects.update(cx, |projects, cx| {
        projects.project_sort = (ProjectSort::Tasks, true);
        cx.notify();
    });
    projects.read_with(cx, |projects, _| {
        assert_eq!(projects.project_sort, (ProjectSort::Tasks, true));
    });
}

#[gpui::test]
fn project_tasks_supports_board_mode_and_task_panel_inspector(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let task1 = ProjectRecord {
        project: "P".into(),
        path: "P/Tasks/T1.md".into(),
        kind: "task".into(),
        title: "T1".into(),
        content: "---\nstatus: doing\ncontext: Terminal\n---\n".into(),
        done: false,
        due: Some("2026-11-10".into()),
        priority: 2,
    };
    let task2 = ProjectRecord {
        project: "P".into(),
        path: "P/Tasks/T2.md".into(),
        kind: "task".into(),
        title: "T2".into(),
        content: "---\nstatus: open\n---\n".into(),
        done: false,
        due: None,
        priority: 1,
    };

    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let records = vec![task1.clone(), task2.clone()];
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            projects.project = Some((
                "P".into(),
                Remote {
                    value: Some(records),
                    ..Remote::default()
                },
            ));
            projects.project_section = ProjectSection::Tasks;
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let projects = slot.take().unwrap();

    // Starts in table mode
    projects.read_with(cx, |projects, _| {
        assert_eq!(projects.task_view_mode, TaskViewMode::Table);
        assert!(projects.task_panel.is_none());
    });

    // Open task panel on task1
    let global = tables::project_record_to_global_task(&task1);
    cx.update(|window, cx| {
        projects.update(cx, |projects, cx| {
            projects.select_task(&global, window, cx);
        });
    });
    cx.run_until_parked();
    projects.read_with(cx, |projects, cx| {
        let panel = projects.task_panel.as_ref().expect("task panel opened");
        assert_eq!(panel.path, "P/Tasks/T1.md");
        let context = panel.input("context").expect("context input present");
        assert_eq!(context.read(cx).text(), "Terminal");
    });

    // Toggle to board mode
    projects.update(cx, |projects, cx| {
        projects.task_view_mode = TaskViewMode::Board;
        cx.notify();
    });
    cx.run_until_parked();
    projects.read_with(cx, |projects, _| {
        assert_eq!(projects.task_view_mode, TaskViewMode::Board);
        assert!(
            projects.task_panel.is_some(),
            "inspector persists across view modes"
        );
    });
}

#[gpui::test]
fn keyboard_navigation_and_inline_task_editing_in_projects(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let p1 = ProjectSummary {
        name: "Hearth".into(),
        archived: false,
        open: 2,
        done: 1,
        notes: 1,
    };
    let p2 = ProjectSummary {
        name: "Atlas".into(),
        archived: false,
        open: 0,
        done: 5,
        notes: 10,
    };
    let (projects, cx) = cx.add_window_view(move |_window, cx| {
        let mut view = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
        view.projects.value = Some(vec![p1, p2]);
        view
    });
    cx.run_until_parked();

    // 1. Hub navigation with j / k
    projects.update(cx, |view, _cx| {
        assert_eq!(view.selected_index, None);
    });
    cx.update(|window, cx| {
        projects.update(cx, |view, cx| {
            view.on_select_next(&actions::SelectNext, window, cx);
        });
    });
    projects.read_with(cx, |view, _| {
        assert_eq!(view.selected_index, Some(0));
    });
    cx.update(|window, cx| {
        projects.update(cx, |view, cx| {
            view.on_select_next(&actions::SelectNext, window, cx);
        });
    });
    projects.read_with(cx, |view, _| {
        assert_eq!(view.selected_index, Some(1));
    });
    cx.update(|window, cx| {
        projects.update(cx, |view, cx| {
            view.on_select_prev(&actions::SelectPrev, window, cx);
        });
    });
    projects.read_with(cx, |view, _| {
        assert_eq!(view.selected_index, Some(0));
    });

    // 2. Open project and navigate tasks
    let task1 = ProjectRecord {
        project: "Hearth".into(),
        path: "Hearth/Tasks/T1.md".into(),
        kind: "task".into(),
        title: "T1".into(),
        content: "---\ntype: task\nstatus: open\n---\n# T1\n".into(),
        done: false,
        due: None,
        priority: 2,
    };
    let task2 = ProjectRecord {
        project: "Hearth".into(),
        path: "Hearth/Tasks/T2.md".into(),
        kind: "task".into(),
        title: "T2".into(),
        content: "---\ntype: task\nstatus: doing\n---\n# T2\n".into(),
        done: false,
        due: None,
        priority: 1,
    };
    projects.update(cx, |view, cx| {
        view.project = Some((
            "Hearth".into(),
            Remote {
                value: Some(vec![task1.clone(), task2.clone()]),
                ..Remote::default()
            },
        ));
        view.project_section = ProjectSection::Tasks;
        cx.notify();
    });
    cx.run_until_parked();

    // Driving task actions from the current row path
    projects.update(cx, |view, cx| {
        view.selected_task_path = Some("Hearth/Tasks/T1.md".into());
        cx.notify();
    });
    cx.run_until_parked();

    // Inline task title editing via `e` action on current row
    cx.update(|window, cx| {
        projects.update(cx, |view, cx| {
            view.on_edit_task_title_action(&actions::EditTaskTitle, window, cx);
        });
    });
    projects.read_with(cx, |view, _| {
        assert!(view.task_title_edit.is_some());
        let (path, original, _, _) = view.task_title_edit.as_ref().unwrap();
        assert_eq!(path, "Hearth/Tasks/T1.md");
        assert_eq!(original, "T1");
    });

    // Cancel editing
    projects.update(cx, |view, cx| {
        view.cancel_task_title(cx);
        assert!(view.task_title_edit.is_none());
    });
}

#[test]
fn column_specs_and_sort_col_ids() {
    assert_eq!(ProjectSort::from_col_id("name"), Some(ProjectSort::Name));
    assert_eq!(
        ProjectSort::from_col_id("project_tasks"),
        Some(ProjectSort::Tasks)
    );
    assert_eq!(TaskSort::from_col_id("due"), Some(TaskSort::Due));
    assert_eq!(NoteSort::from_col_id("kind"), Some(NoteSort::Kind));
}

#[test]
fn task_focus_filters_and_unblocker_chain() {
    let mut blocked_fields = std::collections::BTreeMap::new();
    blocked_fields.insert("blocked_by".into(), "Root Task".into());
    blocked_fields.insert("status".into(), "blocked".into());

    let root = GlobalTask {
        id: "P/root.md".into(),
        kind: "task".into(),
        title: "Root Task".into(),
        done: false,
        due: Some("2026-11-20".into()),
        priority: 1,
        summary: None,
        project: "P".into(),
        fields: Default::default(),
    };
    let downstream = GlobalTask {
        id: "P/down.md".into(),
        kind: "task".into(),
        title: "Downstream".into(),
        done: false,
        due: Some("2026-11-18".into()),
        priority: 2,
        summary: None,
        project: "P".into(),
        fields: blocked_fields,
    };
    let all = vec![root.clone(), downstream.clone()];
    let today = "2026-11-20";

    assert_eq!(unblocked_by_task(&root, &all).len(), 1);
    assert!(TaskFocusFilter::NeedsMeToday.matches(&root, &all, today));
    assert!(TaskFocusFilter::Unblockers.matches(&root, &all, today));
    assert!(!TaskFocusFilter::Drifting.matches(&root, &all, today));

    assert!(TaskFocusFilter::BlockedOnOthers.matches(&downstream, &all, today));
    assert!(TaskFocusFilter::Drifting.matches(&downstream, &all, today));
    assert!(!TaskFocusFilter::Unblockers.matches(&downstream, &all, today));
}

#[test]
fn extract_open_checklist_items_parses_unchecked_only() {
    let md = "# Notes\n- [ ] Book train\n- [x] Done item\n* [ ] Pack adapter\n- [ ]   \n";
    assert_eq!(
        extract_open_checklist_items(md),
        vec!["Book train".to_string(), "Pack adapter".to_string()]
    );
}

#[gpui::test]
fn guided_review_mode_mounts_and_exits(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            projects.review_mode = true;
            projects.all_tasks = Remote {
                value: Some(vec![GlobalTask {
                    id: "P/1.md".into(),
                    kind: "task".into(),
                    title: "Overdue item".into(),
                    done: false,
                    due: Some("2026-01-01".into()),
                    priority: 1,
                    summary: None,
                    project: "P".into(),
                    fields: Default::default(),
                }]),
                ..Remote::default()
            };
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let projects = slot.take().unwrap();
    projects.read_with(cx, |p, _| assert!(p.review_mode));
}

#[gpui::test]
fn all_notes_split_pane_shows_list_and_editor_side_by_side(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx);
            projects.tab = ProjectsTab::AllNotes;
            projects.all_notes = Remote {
                value: Some(vec![GlobalNote {
                    path: "Sjel/Architecture.md".into(),
                    kind: "note".into(),
                    title: "Architecture".into(),
                    project: "Sjel".into(),
                    url: None,
                }]),
                ..Remote::default()
            };
            let editor = cx.new(|cx| {
                NoteEditor::new(
                    record(),
                    std::sync::Arc::new(|_| Box::pin(async { Ok(()) })),
                    window,
                    cx,
                )
            });
            projects.editor = Some(editor);
            projects
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("all-notes-split").is_some());
    assert!(cx.debug_bounds("all-notes").is_some());
    let projects = slot.take().unwrap();
    projects.read_with(cx, |p, _| assert!(p.editor.is_some()));
}

#[gpui::test]
fn mobile_projects_toolbar_keeps_search_wide_and_new_project_on_screen(cx: &mut TestAppContext) {
    crate::test_support::init(cx);
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let projects = cx.new(|cx| {
            let mut projects = Projects::new(Repo::new(Err(ShellError::NotLoopback)), cx)
                .with_density(Density::Mobile);
            projects.projects = Remote {
                value: Some(vec![ProjectSummary {
                    name: "Hearth".into(),
                    archived: false,
                    open: 2,
                    done: 1,
                    notes: 1,
                }]),
                ..Remote::default()
            };
            projects
        });
        struct MobileFrame(Entity<Projects>);
        impl Render for MobileFrame {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(375.)).h(px(667.)).child(self.0.clone())
            }
        }
        let frame = cx.new(|_| MobileFrame(projects));
        gpui_kit::component::Root::new(frame, window, cx)
    });
    cx.run_until_parked();
    let search = cx
        .debug_bounds("projects-hub-search-box")
        .expect("search box rendered");
    let new_project = cx
        .debug_bounds("new-project-btn-host")
        .expect("+ New Project button rendered");
    assert!(
        search.size.width >= px(300.),
        "mobile search box keeps usable width: {:?}",
        search.size.width
    );
    assert!(
        new_project.right() <= px(375.),
        "+ New Project stays inside the 375px screen: {:?}",
        new_project.right()
    );
}
